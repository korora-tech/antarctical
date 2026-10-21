//! The sync worker: one thread that owns credentials, talks to Google and pushes results to
//! the UI. The UI never blocks on the network or the keyring; it sends [`Command`]s and renders
//! whatever [`SyncEvent`]s arrive.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};

use crate::google::api::{self, ApiError};
use crate::google::oauth::{self, AuthError, ClientCredentials, TokenResponse};
use crate::model::{Calendar, Event, add_days, add_months, day_start, first_of_month};
use crate::secrets::Secrets;

/// How often visible data is re-fetched while signed in.
pub const REFRESH_EVERY: Duration = Duration::from_secs(5 * 60);
const RETRY_AFTER_ERROR: Duration = Duration::from_secs(60);
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Deliberately not `Debug`: `SaveClient` carries the client secret.
pub enum Command {
    /// `client_secret: None` keeps the stored secret (only valid for the same client id).
    SaveClient {
        client_id: String,
        client_secret: Option<String>,
    },
    ClearClient,
    SignIn,
    CancelSignIn,
    SignOut,
    Refresh,
    /// The months (first days) the UI is showing. Missing or stale months are fetched.
    Want(Vec<Date>),
    Shutdown,
    #[doc(hidden)]
    SignInFinished(u64, Result<TokenResponse, AuthError>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthState {
    /// The worker has not read the keyring yet.
    Starting,
    /// No OAuth client saved in Settings.
    NeedsClient,
    SignedOut,
    /// Waiting for the browser; `url` is shown in case the browser did not open.
    SigningIn {
        url: String,
    },
    SignedIn {
        account: Option<String>,
    },
    Demo,
}

#[derive(Clone, Debug)]
pub enum SyncEvent {
    Auth(AuthState),
    /// The saved client id (never the secret), for Settings.
    Client(Option<String>),
    Calendars(Vec<Calendar>),
    /// Replaces everything known for `calendar_id` in `month`.
    Month {
        calendar_id: String,
        month: Date,
        events: Vec<Event>,
    },
    Busy(bool),
    Synced(Timestamp),
    /// A persistent problem to show in the banner; `None` clears it.
    Problem(Option<String>),
    /// A one-off message (saved, signed out…).
    Notice(String),
}

#[derive(Clone)]
pub struct SyncHandle {
    tx: mpsc::Sender<Command>,
}

impl SyncHandle {
    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(cmd);
    }

    pub fn from_sender(tx: mpsc::Sender<Command>) -> Self {
        Self { tx }
    }
}

pub fn start_google(secrets: Secrets) -> (SyncHandle, async_channel::Receiver<SyncEvent>) {
    let (tx, rx) = mpsc::channel();
    let (etx, erx) = async_channel::unbounded();
    let self_tx = tx.clone();
    std::thread::Builder::new()
        .name("antarctical-sync".into())
        .spawn(move || Worker::new(secrets, etx, self_tx).run(rx))
        .expect("spawn sync worker");
    (SyncHandle { tx }, erx)
}

/// Why a Google call failed: no usable token, or the API said no.
enum CallError {
    Auth(String),
    Api(ApiError),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::Auth(m) => f.write_str(m),
            CallError::Api(e) => e.fmt(f),
        }
    }
}

struct Access {
    token: String,
    expires: Instant,
}

struct Worker {
    secrets: Secrets,
    events: async_channel::Sender<SyncEvent>,
    self_tx: mpsc::Sender<Command>,
    agent: ureq::Agent,
    client: Option<ClientCredentials>,
    refresh_token: Option<String>,
    access: Option<Access>,
    calendars: Vec<Calendar>,
    /// Month → when it was last fetched for all calendars.
    fetched: BTreeMap<Date, Instant>,
    visible: Vec<Date>,
    last_refresh: Option<Instant>,
    retry_at: Option<Instant>,
    sign_in: Option<(u64, Arc<AtomicBool>)>,
    sign_in_seq: u64,
    auth: AuthState,
    /// The last keyring read failed (locked, no Secret Service); retried on Refresh.
    keyring_failed: bool,
}

impl Worker {
    fn new(
        secrets: Secrets,
        events: async_channel::Sender<SyncEvent>,
        self_tx: mpsc::Sender<Command>,
    ) -> Self {
        Self {
            secrets,
            events,
            self_tx,
            agent: api::agent(),
            client: None,
            refresh_token: None,
            access: None,
            calendars: vec![],
            fetched: BTreeMap::new(),
            visible: vec![],
            last_refresh: None,
            retry_at: None,
            sign_in: None,
            sign_in_seq: 0,
            auth: AuthState::Starting,
            keyring_failed: false,
        }
    }

    fn emit(&self, ev: SyncEvent) {
        let _ = self.events.send_blocking(ev);
    }

    fn set_auth(&mut self, auth: AuthState) {
        if self.auth != auth {
            self.auth = auth.clone();
            self.emit(SyncEvent::Auth(auth));
        }
    }

    fn run(mut self, rx: mpsc::Receiver<Command>) {
        self.load_from_keyring();
        loop {
            let cmd = match rx.recv_timeout(Duration::from_secs(30)) {
                Ok(cmd) => Some(cmd),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            match cmd {
                Some(Command::Shutdown) => break,
                Some(cmd) => self.handle(cmd),
                None => self.tick(),
            }
        }
        if let Some((_, cancel)) = &self.sign_in {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    fn load_from_keyring(&mut self) {
        self.keyring_failed = false;
        match self.secrets.client() {
            Ok(client) => {
                self.emit(SyncEvent::Client(
                    client.as_ref().map(|c| c.client_id.clone()),
                ));
                self.client = client;
            }
            Err(e) => {
                self.keyring_failed = true;
                self.emit(SyncEvent::Problem(Some(format!(
                    "{e}. Antarctical keeps Google credentials only in the system keyring; \
                     unlock it and press Ctrl+R to retry."
                ))));
                self.set_auth(AuthState::NeedsClient);
                return;
            }
        }
        if self.client.is_none() {
            self.set_auth(AuthState::NeedsClient);
            return;
        }
        match self.secrets.refresh_token() {
            Ok(Some(rt)) => {
                self.refresh_token = Some(rt);
                self.set_auth(AuthState::SignedIn { account: None });
                self.refresh_all();
            }
            Ok(None) => self.set_auth(AuthState::SignedOut),
            Err(e) => {
                self.keyring_failed = true;
                self.emit(SyncEvent::Problem(Some(format!(
                    "{e}. Unlock the keyring and press Ctrl+R to retry."
                ))));
                self.set_auth(AuthState::SignedOut);
            }
        }
    }

    fn signed_in(&self) -> bool {
        self.refresh_token.is_some()
    }

    fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::SaveClient {
                client_id,
                client_secret,
            } => self.save_client(client_id, client_secret),
            Command::ClearClient => {
                self.sign_out(true);
                match self.secrets.clear_client() {
                    Ok(()) => {
                        self.client = None;
                        self.emit(SyncEvent::Client(None));
                        self.emit(SyncEvent::Notice(
                            "OAuth client removed from the keyring".into(),
                        ));
                        self.set_auth(AuthState::NeedsClient);
                    }
                    Err(e) => self.emit(SyncEvent::Problem(Some(e.to_string()))),
                }
            }
            Command::SignIn => self.begin_sign_in(),
            Command::CancelSignIn => {
                if let Some((_, cancel)) = self.sign_in.take() {
                    cancel.store(true, Ordering::Relaxed);
                }
                self.set_auth(if self.signed_in() {
                    AuthState::SignedIn {
                        account: self.account(),
                    }
                } else {
                    AuthState::SignedOut
                });
            }
            Command::SignInFinished(seq, result) => self.finish_sign_in(seq, result),
            Command::SignOut => {
                self.sign_out(true);
                self.emit(SyncEvent::Notice("Signed out of Google".into()));
            }
            Command::Refresh => {
                self.retry_at = None;
                if self.keyring_failed {
                    self.load_from_keyring();
                } else {
                    self.refresh_all();
                }
            }
            Command::Want(months) => {
                self.visible = months;
                self.fetch_missing();
            }
            Command::Shutdown => {}
        }
    }

    fn tick(&mut self) {
        if !self.signed_in() {
            return;
        }
        let now = Instant::now();
        if let Some(at) = self.retry_at {
            if now >= at {
                self.retry_at = None;
                self.refresh_all();
            }
            return;
        }
        if self
            .last_refresh
            .is_none_or(|t| now.duration_since(t) >= REFRESH_EVERY)
        {
            self.refresh_all();
        }
    }

    fn account(&self) -> Option<String> {
        self.calendars
            .iter()
            .find(|c| c.primary)
            .map(|c| c.id.clone())
    }

    // ----- client & sign-in ---------------------------------------------------------------

    fn save_client(&mut self, client_id: String, client_secret: Option<String>) {
        let client_id = client_id.trim().to_string();
        let secret = client_secret
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let client_secret = match (secret, &self.client) {
            (Some(s), _) => s,
            (None, Some(existing)) if existing.client_id == client_id => {
                existing.client_secret.clone()
            }
            _ => {
                self.emit(SyncEvent::Problem(Some(
                    "Enter the client secret for this client ID.".into(),
                )));
                return;
            }
        };
        if client_id.is_empty() {
            self.emit(SyncEvent::Problem(Some("Enter a client ID.".into())));
            return;
        }
        let client = ClientCredentials {
            client_id,
            client_secret,
        };
        let changed = self.client.as_ref() != Some(&client);
        if !changed {
            self.emit(SyncEvent::Notice("OAuth client unchanged".into()));
            return;
        }
        if self
            .client
            .as_ref()
            .is_some_and(|c| c.client_id != client.client_id)
        {
            // A refresh token belongs to the client id that issued it; a rotated secret for
            // the same client keeps the session.
            self.sign_out(true);
        }
        match self.secrets.set_client(&client) {
            Ok(()) => {
                self.emit(SyncEvent::Client(Some(client.client_id.clone())));
                self.client = Some(client);
                self.emit(SyncEvent::Problem(None));
                self.emit(SyncEvent::Notice(
                    "OAuth client saved to the keyring".into(),
                ));
                self.set_auth(AuthState::SignedOut);
            }
            Err(e) => self.emit(SyncEvent::Problem(Some(e.to_string()))),
        }
    }

    fn begin_sign_in(&mut self) {
        let Some(client) = self.client.clone() else {
            self.set_auth(AuthState::NeedsClient);
            return;
        };
        if let Some((_, cancel)) = self.sign_in.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        let loopback = match oauth::Loopback::bind() {
            Ok(l) => l,
            Err(e) => {
                self.emit(SyncEvent::Problem(Some(e.to_string())));
                return;
            }
        };
        let pkce = oauth::new_pkce();
        let state = oauth::random_token(24);
        let url = oauth::authorize_url(
            &client.client_id,
            &loopback.redirect_uri,
            &pkce.challenge,
            &state,
        );
        self.sign_in_seq += 1;
        let seq = self.sign_in_seq;
        let cancel = Arc::new(AtomicBool::new(false));
        self.sign_in = Some((seq, cancel.clone()));
        self.emit(SyncEvent::Problem(None));
        self.set_auth(AuthState::SigningIn { url: url.clone() });
        if let Err(e) = open::that_detached(&url) {
            eprintln!("antarctical: could not open a browser: {e}");
        }
        let tx = self.self_tx.clone();
        let agent = self.agent.clone();
        std::thread::Builder::new()
            .name("antarctical-sign-in".into())
            .spawn(move || {
                let result = loopback
                    .wait_for_code(&state, &cancel, SIGN_IN_TIMEOUT)
                    .and_then(|code| {
                        oauth::exchange_code(
                            &agent,
                            &client,
                            &code,
                            &pkce.verifier,
                            &loopback.redirect_uri,
                        )
                    });
                let _ = tx.send(Command::SignInFinished(seq, result));
            })
            .expect("spawn sign-in thread");
    }

    fn finish_sign_in(&mut self, seq: u64, result: Result<TokenResponse, AuthError>) {
        if self.sign_in.as_ref().map(|(s, _)| *s) != Some(seq) {
            // Superseded or cancelled after Google already issued tokens: give them back.
            if let Ok(t) = result {
                oauth::revoke(
                    &self.agent,
                    t.refresh_token.as_ref().unwrap_or(&t.access_token),
                );
            }
            return;
        }
        self.sign_in = None;
        let tokens = match result {
            Ok(t) => t,
            Err(AuthError::Cancelled) => return,
            Err(e) => {
                self.emit(SyncEvent::Problem(Some(e.to_string())));
                self.set_auth(AuthState::SignedOut);
                return;
            }
        };
        let Some(refresh_token) = tokens.refresh_token else {
            self.emit(SyncEvent::Problem(Some(
                "Google did not return a refresh token. Remove Antarctical's access at \
                 myaccount.google.com/permissions and sign in again."
                    .into(),
            )));
            self.set_auth(AuthState::SignedOut);
            return;
        };
        if let Err(e) = self.secrets.set_refresh_token(&refresh_token) {
            self.emit(SyncEvent::Problem(Some(format!(
                "{e}. You are signed in for this session only."
            ))));
        } else {
            self.emit(SyncEvent::Problem(None));
        }
        self.refresh_token = Some(refresh_token);
        self.access = Some(Access {
            token: tokens.access_token,
            expires: Instant::now() + Duration::from_secs(tokens.expires_in),
        });
        self.set_auth(AuthState::SignedIn { account: None });
        self.refresh_all();
    }

    /// Forgets the session. `revoke` also tells Google to invalidate the refresh token.
    fn sign_out(&mut self, revoke: bool) {
        if let Some((_, cancel)) = self.sign_in.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        if let Some(rt) = self.refresh_token.take()
            && revoke
            && !oauth::revoke(&self.agent, &rt)
        {
            self.emit(SyncEvent::Notice(
                "Signed out here, but Google could not be told. Remove access at \
                 myaccount.google.com/permissions."
                    .into(),
            ));
        }
        self.access = None;
        if let Err(e) = self.secrets.clear_refresh_token() {
            self.emit(SyncEvent::Problem(Some(e.to_string())));
        }
        self.calendars.clear();
        self.fetched.clear();
        self.last_refresh = None;
        self.retry_at = None;
        self.emit(SyncEvent::Calendars(vec![]));
        self.set_auth(if self.client.is_some() {
            AuthState::SignedOut
        } else {
            AuthState::NeedsClient
        });
    }

    fn access_token(&mut self) -> Result<String, String> {
        if let Some(a) = &self.access
            && a.expires > Instant::now() + Duration::from_secs(60)
        {
            return Ok(a.token.clone());
        }
        let (Some(client), Some(rt)) = (self.client.clone(), self.refresh_token.clone()) else {
            return Err("Not signed in".into());
        };
        match oauth::refresh(&self.agent, &client, &rt) {
            Ok(t) => {
                if let Some(new_rt) = t.refresh_token
                    && new_rt != rt
                {
                    let _ = self.secrets.set_refresh_token(&new_rt);
                    self.refresh_token = Some(new_rt);
                }
                self.access = Some(Access {
                    token: t.access_token.clone(),
                    expires: Instant::now() + Duration::from_secs(t.expires_in),
                });
                Ok(t.access_token)
            }
            Err(AuthError::InvalidGrant(_)) => {
                self.sign_out(false);
                Err("Your Google sign-in expired or was revoked. Sign in again.".into())
            }
            Err(AuthError::InvalidClient(m)) => {
                self.sign_out(false);
                Err(format!(
                    "Google rejected the OAuth client saved in Settings ({m}). \
                     Check it, then sign in again."
                ))
            }
            Err(e) => Err(e.to_string()),
        }
    }

    /// Runs `f` with a valid token, refreshing once if Google says the token is stale.
    fn with_token<T>(
        &mut self,
        f: impl Fn(&ureq::Agent, &str) -> Result<T, ApiError>,
    ) -> Result<T, CallError> {
        let token = self.access_token().map_err(CallError::Auth)?;
        match f(&self.agent, &token) {
            Err(ApiError::Unauthorized) => {
                self.access = None;
                let token = self.access_token().map_err(CallError::Auth)?;
                f(&self.agent, &token).map_err(CallError::Api)
            }
            other => other.map_err(CallError::Api),
        }
    }

    // ----- fetching -------------------------------------------------------------------------

    /// The months kept fresh even when not on screen (tray agenda and reminders).
    fn core_months() -> Vec<Date> {
        let today = Zoned::now().date();
        let m = first_of_month(today);
        vec![add_months(m, -1), m, add_months(m, 1)]
    }

    fn refresh_all(&mut self) {
        if !self.signed_in() {
            return;
        }
        self.emit(SyncEvent::Busy(true));
        let result = self.with_token(api::calendar_list);
        match result {
            Ok(cals) => {
                self.calendars = cals.clone();
                self.emit(SyncEvent::Calendars(cals));
                self.set_auth(AuthState::SignedIn {
                    account: self.account(),
                });
            }
            Err(e) => {
                self.fail(e.to_string());
                return;
            }
        }
        let months: BTreeSet<Date> = Self::core_months()
            .into_iter()
            .chain(self.visible.iter().copied())
            .collect();
        // Everything else becomes stale and is re-fetched when it is next shown.
        self.fetched.retain(|m, _| months.contains(m));
        if let Some(problems) = self.fetch_months(&months.into_iter().collect::<Vec<_>>()) {
            self.last_refresh = Some(Instant::now());
            self.emit(SyncEvent::Problem(problem_text(&problems)));
            self.emit(SyncEvent::Synced(Timestamp::now()));
        }
        self.emit(SyncEvent::Busy(false));
    }

    fn fetch_missing(&mut self) {
        if !self.signed_in() || self.calendars.is_empty() {
            return;
        }
        let missing: Vec<Date> = self
            .visible
            .iter()
            .copied()
            .filter(|m| {
                self.fetched
                    .get(m)
                    .is_none_or(|t| t.elapsed() >= REFRESH_EVERY)
            })
            .collect();
        if missing.is_empty() {
            return;
        }
        self.emit(SyncEvent::Busy(true));
        if let Some(problems) = self.fetch_months(&missing)
            && !problems.is_empty()
        {
            self.emit(SyncEvent::Problem(problem_text(&problems)));
        }
        self.emit(SyncEvent::Busy(false));
    }

    /// Fetches `months` with one request per calendar per contiguous run of months, and
    /// splits the result into per-month replacements. Returns the per-calendar problems, or
    /// `None` when a failure aborted the sync.
    fn fetch_months(&mut self, months: &[Date]) -> Option<Vec<String>> {
        let tz = TimeZone::system();
        let mut problems = vec![];
        for run in contiguous_runs(months) {
            let (first, last) = (run[0], run[run.len() - 1]);
            // A day of slack each side: all-day dates are matched against local midnight,
            // which can sit up to a day away from the instants Google filters on.
            let from = day_start(add_days(first, -1), &tz).timestamp();
            let to = day_start(add_days(add_months(last, 1), 1), &tz).timestamp();
            for cal in self.calendars.clone() {
                let result =
                    self.with_token(|agent, token| api::events(agent, token, &cal, from, to));
                let events = match result {
                    Ok(evs) => evs,
                    // A calendar-specific refusal: report it and carry on with the others.
                    Err(CallError::Api(
                        e @ (ApiError::Forbidden(_) | ApiError::NotFound | ApiError::Http(..)),
                    )) => {
                        problems.push(format!("{}: {e}", cal.name));
                        continue;
                    }
                    // Auth, network, rate limit or decode trouble: stop and retry later.
                    Err(e) => {
                        self.fail(e.to_string());
                        return None;
                    }
                };
                for &month in &run {
                    let m_from = day_start(month, &tz).timestamp();
                    let m_to = day_start(add_months(month, 1), &tz).timestamp();
                    let in_month = events
                        .iter()
                        .filter(|e| {
                            let (s, en) = e.instants(&tz);
                            if s == en {
                                s >= m_from && s < m_to
                            } else {
                                s < m_to && en > m_from
                            }
                        })
                        .cloned()
                        .collect();
                    self.emit(SyncEvent::Month {
                        calendar_id: cal.id.clone(),
                        month,
                        events: in_month,
                    });
                }
            }
            let now = Instant::now();
            for &m in &run {
                self.fetched.insert(m, now);
            }
        }
        Some(problems)
    }

    fn fail(&mut self, message: String) {
        self.emit(SyncEvent::Problem(Some(message)));
        self.emit(SyncEvent::Busy(false));
        if self.signed_in() {
            self.retry_at = Some(Instant::now() + RETRY_AFTER_ERROR);
        }
    }
}

fn problem_text(problems: &[String]) -> Option<String> {
    (!problems.is_empty()).then(|| format!("Skipped: {}", problems.join("; ")))
}

/// Groups first-of-month dates into runs of consecutive months (sorted, de-duplicated).
fn contiguous_runs(months: &[Date]) -> Vec<Vec<Date>> {
    let mut sorted: Vec<Date> = months.to_vec();
    sorted.sort();
    sorted.dedup();
    let mut runs: Vec<Vec<Date>> = vec![];
    for m in sorted {
        match runs.last_mut() {
            Some(run) if add_months(*run.last().expect("non-empty"), 1) == m => run.push(m),
            _ => runs.push(vec![m]),
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn runs_split_on_gaps() {
        let runs = contiguous_runs(&[
            date(2026, 11, 1),
            date(2020, 1, 1),
            date(2026, 9, 1),
            date(2026, 10, 1),
            date(2026, 10, 1),
            date(2020, 2, 1),
        ]);
        assert_eq!(
            runs,
            vec![
                vec![date(2020, 1, 1), date(2020, 2, 1)],
                vec![date(2026, 9, 1), date(2026, 10, 1), date(2026, 11, 1)],
            ]
        );
        assert!(contiguous_runs(&[]).is_empty());
    }
}
