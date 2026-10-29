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
use crate::model;
use crate::model::{Calendar, Event};
use crate::secrets::{SecretError, Secrets};

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
    /// The user dismissed the banner (including sticky "account dropped" messages).
    DismissProblems,
    /// The months (first days) a UI surface is showing. Missing or stale months are fetched.
    Want(ViewId, Vec<Date>),
    /// A UI surface closed; its months no longer need to be kept fresh.
    Forget(ViewId),
    Shutdown,
    #[doc(hidden)]
    SignInFinished(u64, Result<TokenResponse, AuthError>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ViewId {
    Main,
    Tray,
    /// A desktop shell (Dux) reading events over the dankcal socket (see `dankcal.rs`). Its
    /// months load without the busy indicator, and are forgotten when it disconnects.
    Shell,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthState {
    /// The worker has not read the keyring yet.
    Starting,
    /// The keyring could not be read (locked, prompt dismissed, no Secret Service). Nothing is
    /// known about the client or accounts until it is retried (Refresh).
    Locked,
    /// The client (saved or built-in) is known; see [`SyncEvent::Accounts`] for who is signed in.
    Ready,
    Demo,
}

#[derive(Clone, Debug)]
pub enum SyncEvent {
    Auth(AuthState),
    /// The client id saved in Settings (never the secret); `None` means the built-in client.
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

pub fn start_google(
    secrets: Secrets,
) -> std::io::Result<(SyncHandle, async_channel::Receiver<SyncEvent>)> {
    let (tx, rx) = mpsc::channel();
    let (etx, erx) = async_channel::unbounded();
    let self_tx = tx.clone();
    std::thread::Builder::new()
        .name("antarctical-sync".into())
        .spawn(move || Worker::new(secrets, etx, self_tx).run(rx))?;
    Ok((SyncHandle { tx }, erx))
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

struct PendingSignIn {
    seq: u64,
    cancel: Arc<AtomicBool>,
}

#[derive(Clone, Copy)]
enum Revoke {
    WithGoogle,
    LocalOnly,
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
    visible: BTreeMap<ViewId, Vec<Date>>,
    last_refresh: Option<Instant>,
    retry_at: Option<Instant>,
    sign_in: Option<PendingSignIn>,
    sign_in_seq: u64,
    auth: AuthState,
    /// The last keyring read failed (locked, no Secret Service); retried on Refresh.
    keyring_failed: bool,
    /// Messages that outlive a sync (an account was dropped, a migration failed), keyed so
    /// re-adding the account clears its message. Shown until dismissed.
    sticky: BTreeMap<String, String>,
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
            visible: BTreeMap::new(),
            last_refresh: None,
            retry_at: None,
            sign_in: None,
            sign_in_seq: 0,
            auth: AuthState::Starting,
            keyring_failed: false,
            sticky: BTreeMap::new(),
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

    /// Sets the banner to the sticky messages plus this run's `problems`.
    fn report(&self, problems: &[String]) {
        let all: Vec<String> = self
            .sticky
            .values()
            .cloned()
            .chain(problems.iter().cloned())
            .collect();
        self.emit(SyncEvent::Problem(
            (!all.is_empty()).then(|| all.join(" · ")),
        ));
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
        // A command read while coalescing `Want`s, handled next.
        let mut pending = None;
        loop {
            let cmd = match pending.take() {
                Some(cmd) => Some(cmd),
                None => match rx.recv_timeout(Duration::from_secs(30)) {
                    Ok(cmd) => Some(cmd),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                },
            };
            match cmd {
                Some(Command::Shutdown) => break,
                // Of queued requests from the same surface, only the latest matters.
                Some(Command::Want(view, mut months)) => {
                    while let Ok(next) = rx.try_recv() {
                        match next {
                            Command::Want(v, m) if v == view => months = m,
                            other => {
                                pending = Some(other);
                                break;
                            }
                        }
                    }
                    self.handle(Command::Want(view, months));
                }
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
                self.client = Some(client.unwrap_or_else(oauth::builtin_client));
            }
            Err(e) => {
                self.locked(e);
                return;
            }
        }
        let tokens = match self.secrets.tokens() {
            Ok(t) => t,
            // Unreadable, so unusable: start afresh rather than failing forever.
            Err(SecretError::Corrupt(e)) => {
                let _ = self.secrets.clear_tokens();
                self.sticky.insert(
                    "tokens".into(),
                    format!(
                        "Stored sign-ins were unreadable ({e}) and were reset; \
                         add your accounts again."
                    ),
                );
                BTreeMap::new()
            }
            Err(e) => {
                self.locked(e);
                return;
            }
        };
        let mut accounts: Vec<Account> = tokens
            .into_iter()
            .map(|(email, refresh_token)| Account {
                email,
                refresh_token,
                access: None,
                calendars: vec![],
            })
            .collect();
        // Accounts added while the keyring was unavailable live for this session only; a
        // reload must not forget them.
        for a in std::mem::take(&mut self.accounts) {
            if !accounts.iter().any(|b| b.email == a.email) {
                accounts.push(a);
            }
        }
        self.accounts = accounts;
        self.emit_accounts();
        self.set_auth(AuthState::Ready);
        if self.migrate_legacy_token() {
            self.emit_accounts();
        }
        self.report(&[]);
        self.refresh_all();
    }

    fn locked(&mut self, e: SecretError) {
        self.keyring_failed = true;
        self.emit(SyncEvent::Problem(Some(format!(
            "{e}. Antarctical keeps Google credentials only in the system keyring; unlock it \
             and press Ctrl+R to retry."
        ))));
        self.set_auth(AuthState::Locked);
    }

    /// A single-account build stored one refresh token without its account. Find out whose
    /// it is and move it into the per-account map.
    fn migrate_legacy_token(&mut self) -> bool {
        let rt = match self.secrets.legacy_token() {
            Ok(Some(rt)) => rt,
            Ok(None) => return false,
            Err(e) => {
                self.sticky.insert(
                    "legacy".into(),
                    format!("Could not read the old sign-in: {e}"),
                );
                return false;
            }
        };
        let Some(client) = self.client.clone() else {
            return false;
        };
        let t = match oauth::refresh(&self.agent, &client, &rt) {
            Ok(t) => t,
            // Revoked, or issued to another client: unusable.
            Err(AuthError::InvalidGrant(_) | AuthError::InvalidClient(_)) => {
                let _ = self.secrets.delete_legacy_token();
                return false;
            }
            // Offline or similar: try again next start.
            Err(e) => {
                self.sticky.insert(
                    "legacy".into(),
                    format!("Could not restore the previously signed-in account: {e}"),
                );
                return false;
            }
        };
        let email = match api::primary_email(&self.agent, &t.access_token) {
            Ok(email) => email,
            Err(e) => {
                self.sticky.insert(
                    "legacy".into(),
                    format!("Could not identify the previously signed-in account: {e}"),
                );
                return false;
            }
        };
        self.sticky.remove("legacy");
        // The per-account map is newer than the single-account item; never overwrite it.
        if self.accounts.iter().any(|a| a.email == email) {
            let _ = self.secrets.delete_legacy_token();
            return false;
        }
        if self.secrets.set_token(&email, &rt).is_ok() {
            let _ = self.secrets.delete_legacy_token();
        }
        self.accounts.push(Account {
            email,
            refresh_token: rt,
            access: Some(Access {
                token: t.access_token,
                expires: Instant::now() + Duration::from_secs(t.expires_in),
            }),
            calendars: vec![],
        });
        true
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
                self.remove_all_accounts(Revoke::WithGoogle);
                match self.secrets.clear_client() {
                    Ok(()) => {
                        // Tokens from the removed client were revoked above; the built-in
                        // client starts with no accounts.
                        self.client = Some(oauth::builtin_client());
                        self.sticky.remove("client");
                        self.report(&[]);
                        self.emit(SyncEvent::Client(None));
                        self.emit(SyncEvent::Notice(
                            "Your OAuth client was removed; using the built-in client".into(),
                        ));
                        self.set_auth(AuthState::Ready);
                    }
                    Err(e) => self.emit(SyncEvent::Problem(Some(e.to_string()))),
                }
            }
            Command::AddAccount => self.begin_sign_in(),
            Command::CancelSignIn => self.cancel_sign_in(),
            Command::SignInFinished(seq, result) => self.finish_sign_in(seq, result),
            Command::RemoveAccount(email) => {
                if self.remove_account(&email, Revoke::WithGoogle) {
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
            Command::DismissProblems => {
                self.sticky.clear();
                self.emit(SyncEvent::Problem(None));
            }
            Command::Want(view, months) => {
                self.visible.insert(view, months);
                self.fetch_missing();
            }
            Command::Forget(view) => {
                self.visible.remove(&view);
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
            self.remove_all_accounts(Revoke::WithGoogle);
        }
        match self.secrets.set_client(&client) {
            Ok(()) => {
                self.emit(SyncEvent::Client(Some(client.client_id.clone())));
                self.client = Some(client);
                self.sticky.remove("client");
                self.report(&[]);
                self.emit(SyncEvent::Notice(
                    "OAuth client saved to the keyring".into(),
                ));
                self.set_auth(AuthState::Ready);
            }
            Err(e) => self.emit(SyncEvent::Problem(Some(e.to_string()))),
        }
    }

    fn cancel_sign_in(&mut self) {
        if let Some(sign_in) = self.sign_in.take() {
            sign_in.cancel.store(true, Ordering::Relaxed);
            self.emit(SyncEvent::SigningIn(None));
        }
    }

    fn begin_sign_in(&mut self) {
        // No client until the keyring has been read: a saved client must not be bypassed.
        let Some(client) = self.client.clone() else {
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
        self.sign_in = Some(PendingSignIn {
            seq,
            cancel: cancel.clone(),
        });
        self.emit(SyncEvent::Problem(None));
        self.emit(SyncEvent::SigningIn(Some(url.clone())));
        if let Err(e) = open::that_detached(&url) {
            eprintln!("antarctical: could not open a browser: {e}");
        }
        let tx = self.self_tx.clone();
        let agent = self.agent.clone();
        let result = std::thread::Builder::new()
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
            });
        if let Err(error) = result {
            self.sign_in = None;
            self.emit(SyncEvent::SigningIn(None));
            self.emit(SyncEvent::Problem(Some(format!(
                "Could not start the sign-in worker: {error}"
            ))));
        }
    }

    fn finish_sign_in(&mut self, seq: u64, result: Result<TokenResponse, AuthError>) {
        // Superseded or cancelled: drop whatever Google issued. Never revoke a token whose
        // account is unknown: revoking removes the app's whole grant for that user, which
        // would also sign out an account that is already here.
        if self.sign_in.as_ref().map(|sign_in| sign_in.seq) != Some(seq) {
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
                self.emit(SyncEvent::Problem(Some(format!(
                    "Signed in, but could not read the account's primary calendar: {e}"
                ))));
                return;
            }
        };
        self.sticky.remove(&email);
        if let Err(e) = self.secrets.set_token(&email, &refresh_token) {
            self.sticky.insert(
                email.clone(),
                format!("{e}. {email} is signed in for this session only."),
            );
        }
        self.report(&[]);
        let access = Some(Access {
            token: tokens.access_token,
            expires: Instant::now() + Duration::from_secs(tokens.expires_in),
        });
        match self.accounts.iter_mut().find(|a| a.email == email) {
            // Signing in again with an account we have: the new token replaces the old one.
            // The old one is not revoked: that would revoke the whole grant, new token included.
            Some(existing) => {
                existing.refresh_token = refresh_token;
                existing.access = access;
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

    /// Forgets one account. `Revoke::WithGoogle` also invalidates its refresh token with Google.
    /// Returns whether the account existed.
    fn remove_account(&mut self, email: &str, revoke: Revoke) -> bool {
        let Some(i) = self.accounts.iter().position(|a| a.email == email) else {
            return false;
        };
        let account = self.accounts.remove(i);
        if matches!(revoke, Revoke::WithGoogle)
            && !oauth::revoke(&self.agent, &account.refresh_token)
        {
            self.emit(SyncEvent::Notice(format!(
                "Signed out of {email} here, but Google could not be told. Remove access at \
                 myaccount.google.com/permissions."
            )));
        }
        if let Err(e) = self.secrets.remove_token(email) {
            self.sticky.insert(email.to_string(), e.to_string());
            self.report(&[]);
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

    fn remove_all_accounts(&mut self, revoke: Revoke) {
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
                self.remove_account(email, Revoke::LocalOnly);
                let msg = format!(
                    "The sign-in for {email} expired or was revoked; add it again in Settings."
                );
                self.sticky.insert(email.to_string(), msg.clone());
                Err(CallError::Auth(msg))
            }
            Err(AuthError::InvalidClient(m)) => {
                // No account can work with a rejected client.
                self.remove_all_accounts(Revoke::LocalOnly);
                let msg = format!(
                    "Google rejected the OAuth client ({m}). If you saved your own in \
                     Settings, check it; then add your accounts again."
                );
                self.sticky.insert("client".into(), msg.clone());
                Err(CallError::Auth(msg))
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
        let m = model::first_of_month(today);
        vec![model::add_months(m, -1), m, model::add_months(m, 1)]
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
                // Signed out just now (its message is sticky); the others carry on.
                Err(CallError::Gone) => {}
                Err(CallError::Auth(_)) if !self.accounts.iter().any(|a| a.email == email) => {}
                // This account is refused (quota, Workspace policy…); the others carry on.
                Err(CallError::Api(
                    e @ (ApiError::Forbidden(_) | ApiError::NotFound | ApiError::Http { .. }),
                )) => problems.push(format!("{email}: {e}")),
                Err(e) => {
                    self.emit_calendars();
                    self.fail(e.to_string());
                    return;
                }
            }
        }
        self.emit_calendars();
        let months: BTreeSet<Date> = Self::core_months()
            .into_iter()
            .chain(self.visible.values().flatten().copied())
            .collect();
        // Everything else becomes stale and is re-fetched when it is next shown.
        self.fetched.retain(|m, _| months.contains(m));
        if let Some(more) = self.fetch_months(&months.into_iter().collect::<Vec<_>>()) {
            problems.extend(more);
            self.last_refresh = Some(Instant::now());
            self.report(&problems);
            self.emit(SyncEvent::Synced(Timestamp::now()));
        }
        self.emit(SyncEvent::Busy(false));
    }

    fn fetch_missing(&mut self) {
        if !self.accounts.iter().any(|a| !a.calendars.is_empty()) {
            return;
        }
        let wanted: BTreeSet<Date> = self.visible.values().flatten().copied().collect();
        let missing: Vec<Date> = wanted
            .into_iter()
            .filter(|m| {
                self.fetched
                    .get(m)
                    .is_none_or(|t| t.elapsed() >= REFRESH_EVERY)
            })
            .collect();
        if missing.is_empty() {
            return;
        }
        // Months only the shell wants load quietly, without the UI's busy indicator.
        let busy = missing.iter().any(|m| {
            self.visible
                .iter()
                .any(|(view, months)| *view != ViewId::Shell && months.contains(m))
        });
        if busy {
            self.emit(SyncEvent::Busy(true));
        }
        if let Some(problems) = self.fetch_months(&missing)
            && !problems.is_empty()
        {
            self.report(&problems);
        }
        if busy {
            self.emit(SyncEvent::Busy(false));
        }
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
            let from = model::day_start(model::add_days(first, -1), &tz).timestamp();
            let to =
                model::day_start(model::add_days(model::add_months(last, 1), 1), &tz).timestamp();
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
                        e @ (ApiError::Forbidden(_) | ApiError::NotFound | ApiError::Http { .. }),
                    )) => {
                        problems.push(format!("{}: {e}", cal.name));
                        continue;
                    }
                    Err(CallError::Gone) => continue,
                    // This account was just signed out (its message is sticky).
                    Err(CallError::Auth(_)) if !self.accounts.iter().any(|a| a.email == email) => {
                        continue;
                    }
                    // Network, rate limit or decode trouble: stop and retry later.
                    Err(e) => {
                        self.fail(e.to_string());
                        return None;
                    }
                };
                for &month in &run {
                    let m_from = model::day_start(month, &tz).timestamp();
                    let m_to = model::day_start(model::add_months(month, 1), &tz).timestamp();
                    let in_month = events
                        .iter()
                        .filter(|e| {
                            let span = e.instants(&tz);
                            if span.start == span.end {
                                span.start >= m_from && span.start < m_to
                            } else {
                                span.start < m_to && span.end > m_from
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
        self.report(&[message]);
        self.emit(SyncEvent::Busy(false));
        if self.signed_in() {
            self.retry_at = Some(Instant::now() + RETRY_AFTER_ERROR);
        }
    }
}

/// Groups first-of-month dates into runs of consecutive months (sorted, de-duplicated).
fn contiguous_runs(months: &[Date]) -> Vec<Vec<Date>> {
    let mut sorted: Vec<Date> = months.to_vec();
    sorted.sort();
    sorted.dedup();
    let mut runs: Vec<Vec<Date>> = vec![];
    for m in sorted {
        match runs.last_mut() {
            Some(run)
                if run
                    .last()
                    .is_some_and(|last| model::add_months(*last, 1) == m) =>
            {
                run.push(m)
            }
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
