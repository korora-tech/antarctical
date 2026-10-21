//! The sync worker: one thread that owns credentials, talks to Google and pushes results to
//! the UI. The UI never blocks on the network or the keyring; it sends [`Command`]s and renders
//! whatever [`SyncEvent`]s arrive.
//!
//! One OAuth client serves any number of Google accounts. Each account has its own refresh
//! token (in the keyring), access token (in memory) and calendars; a failure in one account
//! does not stop the others.

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
    /// Removes the client and every account.
    ClearClient,
    /// Starts a browser sign-in; the account chosen there is added (or re-authorised).
    AddAccount,
    CancelSignIn,
    /// Signs one account out: revokes its token and forgets it.
    RemoveAccount(String),
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
    /// A client is saved; see [`SyncEvent::Accounts`] for who is signed in.
    Ready,
    Demo,
}

#[derive(Clone, Debug)]
pub enum SyncEvent {
    Auth(AuthState),
    /// The saved client id (never the secret), for Settings.
    Client(Option<String>),
    /// Signed-in account emails, in display order.
    Accounts(Vec<String>),
    /// A browser sign-in is waiting (`Some(url)`, shown in case no browser opened) or not.
    SigningIn(Option<String>),
    /// Every calendar of every account (see [`Calendar::account`]).
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

/// Why a Google call failed.
enum CallError {
    /// The account has no usable token (it was just signed out, or the client is rejected).
    Auth(String),
    /// The account is no longer signed in at all; skip it quietly.
    Gone,
    Api(ApiError),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::Auth(m) => f.write_str(m),
            CallError::Gone => f.write_str("Account signed out"),
            CallError::Api(e) => e.fmt(f),
        }
    }
}

struct Access {
    token: String,
    expires: Instant,
}

struct Account {
    email: String,
    refresh_token: String,
    access: Option<Access>,
    calendars: Vec<Calendar>,
}

struct Worker {
    secrets: Secrets,
    events: async_channel::Sender<SyncEvent>,
    self_tx: mpsc::Sender<Command>,
    agent: ureq::Agent,
    client: Option<ClientCredentials>,
    accounts: Vec<Account>,
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
            accounts: vec![],
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

    fn emit_accounts(&self) {
        self.emit(SyncEvent::Accounts(
            self.accounts.iter().map(|a| a.email.clone()).collect(),
        ));
    }

    fn emit_calendars(&self) {
        self.emit(SyncEvent::Calendars(
            self.accounts
                .iter()
                .flat_map(|a| a.calendars.iter().cloned())
                .collect(),
        ));
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
        self.cancel_sign_in();
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
        let tokens = match self.secrets.tokens() {
            Ok(t) => t,
            Err(e) => {
                self.keyring_failed = true;
                self.emit(SyncEvent::Problem(Some(format!(
                    "{e}. Unlock the keyring and press Ctrl+R to retry."
                ))));
                self.set_auth(AuthState::Ready);
                return;
            }
        };
        self.accounts = tokens
            .into_iter()
            .map(|(email, refresh_token)| Account {
                email,
                refresh_token,
                access: None,
                calendars: vec![],
            })
            .collect();
        self.migrate_legacy_token();
        self.emit_accounts();
        self.set_auth(AuthState::Ready);
        self.refresh_all();
    }

    /// A single-account build stored one refresh token without its account. Find out whose
    /// it is and move it into the per-account map.
    fn migrate_legacy_token(&mut self) {
        let (Ok(Some(rt)), Some(client)) = (self.secrets.legacy_token(), self.client.clone())
        else {
            return;
        };
        let t = match oauth::refresh(&self.agent, &client, &rt) {
            Ok(t) => t,
            // Revoked, or issued to another client: unusable.
            Err(AuthError::InvalidGrant(_) | AuthError::InvalidClient(_)) => {
                let _ = self.secrets.delete_legacy_token();
                return;
            }
            // Offline or similar: try again next start.
            Err(e) => {
                self.emit(SyncEvent::Problem(Some(format!(
                    "Could not restore the previously signed-in account: {e}"
                ))));
                return;
            }
        };
        let email = match api::primary_email(&self.agent, &t.access_token) {
            Ok(email) => email,
            Err(e) => {
                self.emit(SyncEvent::Problem(Some(format!(
                    "Could not identify the previously signed-in account: {e}"
                ))));
                return;
            }
        };
        if self.secrets.set_token(&email, &rt).is_ok() {
            let _ = self.secrets.delete_legacy_token();
        }
        self.accounts.retain(|a| a.email != email);
        self.accounts.push(Account {
            email,
            refresh_token: rt,
            access: Some(Access {
                token: t.access_token,
                expires: Instant::now() + Duration::from_secs(t.expires_in),
            }),
            calendars: vec![],
        });
    }

    fn signed_in(&self) -> bool {
        !self.accounts.is_empty()
    }

    fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::SaveClient {
                client_id,
                client_secret,
            } => self.save_client(client_id, client_secret),
            Command::ClearClient => {
                self.remove_all_accounts(true);
                match self.secrets.clear_client() {
                    Ok(()) => {
                        self.client = None;
                        self.emit(SyncEvent::Client(None));
                        self.emit(SyncEvent::Notice(
                            "OAuth client and accounts removed from the keyring".into(),
                        ));
                        self.set_auth(AuthState::NeedsClient);
                    }
                    Err(e) => self.emit(SyncEvent::Problem(Some(e.to_string()))),
                }
            }
            Command::AddAccount => self.begin_sign_in(),
            Command::CancelSignIn => self.cancel_sign_in(),
            Command::SignInFinished(seq, result) => self.finish_sign_in(seq, result),
            Command::RemoveAccount(email) => {
                if self.remove_account(&email, true) {
                    self.emit(SyncEvent::Notice(format!("Signed out of {email}")));
                }
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

    // ----- client & accounts --------------------------------------------------------------

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
        if self.client.as_ref() == Some(&client) {
            self.emit(SyncEvent::Notice("OAuth client unchanged".into()));
            return;
        }
        if self
            .client
            .as_ref()
            .is_some_and(|c| c.client_id != client.client_id)
        {
            // Refresh tokens belong to the client id that issued them; a rotated secret for
            // the same client keeps every session.
            self.remove_all_accounts(true);
        }
        match self.secrets.set_client(&client) {
            Ok(()) => {
                self.emit(SyncEvent::Client(Some(client.client_id.clone())));
                self.client = Some(client);
                self.emit(SyncEvent::Problem(None));
                self.emit(SyncEvent::Notice(
                    "OAuth client saved to the keyring".into(),
                ));
                self.set_auth(AuthState::Ready);
            }
            Err(e) => self.emit(SyncEvent::Problem(Some(e.to_string()))),
        }
    }

    fn cancel_sign_in(&mut self) {
        if let Some((_, cancel)) = self.sign_in.take() {
            cancel.store(true, Ordering::Relaxed);
            self.emit(SyncEvent::SigningIn(None));
        }
    }

    fn begin_sign_in(&mut self) {
        let Some(client) = self.client.clone() else {
            self.set_auth(AuthState::NeedsClient);
            return;
        };
        self.cancel_sign_in();
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
        self.emit(SyncEvent::SigningIn(Some(url.clone())));
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
        self.emit(SyncEvent::SigningIn(None));
        let tokens = match result {
            Ok(t) => t,
            Err(AuthError::Cancelled) => return,
            Err(e) => {
                self.emit(SyncEvent::Problem(Some(e.to_string())));
                return;
            }
        };
        let Some(refresh_token) = tokens.refresh_token else {
            oauth::revoke(&self.agent, &tokens.access_token);
            self.emit(SyncEvent::Problem(Some(
                "Google did not return a refresh token. Remove Antarctical's access at \
                 myaccount.google.com/permissions and sign in again."
                    .into(),
            )));
            return;
        };
        let email = match api::primary_email(&self.agent, &tokens.access_token) {
            Ok(email) => email,
            Err(e) => {
                oauth::revoke(&self.agent, &refresh_token);
                self.emit(SyncEvent::Problem(Some(format!(
                    "Signed in, but could not read the account's primary calendar: {e}"
                ))));
                return;
            }
        };
        if let Err(e) = self.secrets.set_token(&email, &refresh_token) {
            self.emit(SyncEvent::Problem(Some(format!(
                "{e}. {email} is signed in for this session only."
            ))));
        } else {
            self.emit(SyncEvent::Problem(None));
        }
        let access = Some(Access {
            token: tokens.access_token,
            expires: Instant::now() + Duration::from_secs(tokens.expires_in),
        });
        match self.accounts.iter_mut().find(|a| a.email == email) {
            // Signing in again with an account we have: the new token replaces the old one.
            Some(existing) => {
                let old = std::mem::replace(&mut existing.refresh_token, refresh_token);
                existing.access = access;
                if old != existing.refresh_token {
                    oauth::revoke(&self.agent, &old);
                }
                self.emit(SyncEvent::Notice(format!("{email} signed in again")));
            }
            None => {
                self.accounts.push(Account {
                    email: email.clone(),
                    refresh_token,
                    access,
                    calendars: vec![],
                });
                self.emit(SyncEvent::Notice(format!("Added {email}")));
            }
        }
        self.emit_accounts();
        // Months were fetched for the old set of accounts.
        self.fetched.clear();
        self.refresh_all();
    }

    /// Forgets one account. `revoke` also tells Google to invalidate its refresh token.
    /// Returns whether the account existed.
    fn remove_account(&mut self, email: &str, revoke: bool) -> bool {
        let Some(i) = self.accounts.iter().position(|a| a.email == email) else {
            return false;
        };
        let account = self.accounts.remove(i);
        if revoke && !oauth::revoke(&self.agent, &account.refresh_token) {
            self.emit(SyncEvent::Notice(format!(
                "Signed out of {email} here, but Google could not be told. Remove access at \
                 myaccount.google.com/permissions."
            )));
        }
        if let Err(e) = self.secrets.remove_token(email) {
            self.emit(SyncEvent::Problem(Some(e.to_string())));
        }
        if self.accounts.is_empty() {
            self.fetched.clear();
            self.last_refresh = None;
            self.retry_at = None;
        }
        self.emit_accounts();
        self.emit_calendars();
        true
    }

    fn remove_all_accounts(&mut self, revoke: bool) {
        self.cancel_sign_in();
        let emails: Vec<String> = self.accounts.iter().map(|a| a.email.clone()).collect();
        for email in emails {
            self.remove_account(&email, revoke);
        }
        if let Err(e) = self.secrets.clear_tokens() {
            self.emit(SyncEvent::Problem(Some(e.to_string())));
        }
    }

    fn access_token(&mut self, email: &str) -> Result<String, CallError> {
        let client = self.client.clone().ok_or(CallError::Gone)?;
        let account = self
            .accounts
            .iter()
            .find(|a| a.email == email)
            .ok_or(CallError::Gone)?;
        if let Some(a) = &account.access
            && a.expires > Instant::now() + Duration::from_secs(60)
        {
            return Ok(a.token.clone());
        }
        let rt = account.refresh_token.clone();
        match oauth::refresh(&self.agent, &client, &rt) {
            Ok(t) => {
                let account = self
                    .accounts
                    .iter_mut()
                    .find(|a| a.email == email)
                    .ok_or(CallError::Gone)?;
                if let Some(new_rt) = t.refresh_token
                    && new_rt != rt
                {
                    let _ = self.secrets.set_token(email, &new_rt);
                    account.refresh_token = new_rt;
                }
                account.access = Some(Access {
                    token: t.access_token.clone(),
                    expires: Instant::now() + Duration::from_secs(t.expires_in),
                });
                Ok(t.access_token)
            }
            Err(AuthError::InvalidGrant(_)) => {
                self.remove_account(email, false);
                Err(CallError::Auth(format!(
                    "The sign-in for {email} expired or was revoked; add it again in Settings."
                )))
            }
            Err(AuthError::InvalidClient(m)) => {
                // No account can work with a rejected client.
                self.remove_all_accounts(false);
                Err(CallError::Auth(format!(
                    "Google rejected the OAuth client saved in Settings ({m}). \
                     Check it, then add your accounts again."
                )))
            }
            Err(e) => Err(CallError::Auth(e.to_string())),
        }
    }

    /// Runs `f` with a valid token for `email`, refreshing once if Google says it is stale.
    fn with_token<T>(
        &mut self,
        email: &str,
        f: impl Fn(&ureq::Agent, &str) -> Result<T, ApiError>,
    ) -> Result<T, CallError> {
        let token = self.access_token(email)?;
        match f(&self.agent, &token) {
            Err(ApiError::Unauthorized) => {
                if let Some(a) = self.accounts.iter_mut().find(|a| a.email == email) {
                    a.access = None;
                }
                let token = self.access_token(email)?;
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

    fn emails(&self) -> Vec<String> {
        self.accounts.iter().map(|a| a.email.clone()).collect()
    }

    fn refresh_all(&mut self) {
        if !self.signed_in() {
            return;
        }
        self.emit(SyncEvent::Busy(true));
        let mut problems = vec![];
        for email in self.emails() {
            match self.with_token(&email, |agent, token| {
                api::calendar_list(agent, token, &email)
            }) {
                Ok(cals) => {
                    if let Some(a) = self.accounts.iter_mut().find(|a| a.email == email) {
                        a.calendars = cals;
                    }
                }
                Err(CallError::Gone) => {}
                // This account was signed out (expired token); the others carry on.
                Err(CallError::Auth(m)) if !self.accounts.iter().any(|a| a.email == email) => {
                    problems.push(m)
                }
                Err(e) => {
                    self.fail(e.to_string());
                    return;
                }
            }
        }
        self.emit_calendars();
        let months: BTreeSet<Date> = Self::core_months()
            .into_iter()
            .chain(self.visible.iter().copied())
            .collect();
        // Everything else becomes stale and is re-fetched when it is next shown.
        self.fetched.retain(|m, _| months.contains(m));
        if let Some(more) = self.fetch_months(&months.into_iter().collect::<Vec<_>>()) {
            problems.extend(more);
            self.last_refresh = Some(Instant::now());
            self.emit(SyncEvent::Problem(problem_text(&problems)));
            self.emit(SyncEvent::Synced(Timestamp::now()));
        }
        self.emit(SyncEvent::Busy(false));
    }

    fn fetch_missing(&mut self) {
        if !self.accounts.iter().any(|a| !a.calendars.is_empty()) {
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
    /// splits the result into per-month replacements. Returns the per-calendar and
    /// per-account problems, or `None` when a failure aborted the sync.
    fn fetch_months(&mut self, months: &[Date]) -> Option<Vec<String>> {
        let tz = TimeZone::system();
        let mut problems = vec![];
        for run in contiguous_runs(months) {
            let (first, last) = (run[0], run[run.len() - 1]);
            // A day of slack each side: all-day dates are matched against local midnight,
            // which can sit up to a day away from the instants Google filters on.
            let from = day_start(add_days(first, -1), &tz).timestamp();
            let to = day_start(add_days(add_months(last, 1), 1), &tz).timestamp();
            let work: Vec<(String, Calendar)> = self
                .accounts
                .iter()
                .flat_map(|a| a.calendars.iter().map(|c| (a.email.clone(), c.clone())))
                .collect();
            for (email, cal) in work {
                let result = self.with_token(&email, |agent, token| {
                    api::events(agent, token, &cal, from, to)
                });
                let events = match result {
                    Ok(evs) => evs,
                    // A calendar-specific refusal: report it and carry on with the others.
                    Err(CallError::Api(
                        e @ (ApiError::Forbidden(_) | ApiError::NotFound | ApiError::Http(..)),
                    )) => {
                        problems.push(format!("{}: {e}", cal.name));
                        continue;
                    }
                    Err(CallError::Gone) => continue,
                    // This account was just signed out; its calendars are gone from the UI.
                    Err(CallError::Auth(m)) if !self.accounts.iter().any(|a| a.email == email) => {
                        if !problems.contains(&m) {
                            problems.push(m);
                        }
                        continue;
                    }
                    // Network, rate limit or decode trouble: stop and retry later.
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
