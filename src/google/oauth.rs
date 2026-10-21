//! OAuth 2.0 for installed apps: authorization code + PKCE with a loopback redirect
//! (<https://developers.google.com/identity/protocols/oauth2/native-app>).
//!
//! The browser is sent to Google, Google redirects to `http://127.0.0.1:<port>/`, and the one
//! request carrying `code` (or `error`) ends the wait. Nothing here persists anything.

use std::fmt;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use sha2::{Digest, Sha256};

pub const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";
pub const SCOPE: &str = "https://www.googleapis.com/auth/calendar.readonly";

/// A Google "Desktop app" OAuth client. Google requires the secret for desktop clients even
/// though it cannot be kept confidential; it is still treated as a secret locally.
#[derive(Clone, PartialEq, Eq)]
pub struct ClientCredentials {
    pub client_id: String,
    pub client_secret: String,
}

impl fmt::Debug for ClientCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientCredentials")
            .field("client_id", &self.client_id)
            .field("client_secret", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthError {
    /// Google rejected the client id or secret.
    InvalidClient(String),
    /// The refresh token was revoked or expired; the user has to sign in again.
    InvalidGrant(String),
    /// The user declined consent, or Google returned an `error` on the redirect.
    Denied(String),
    Cancelled,
    TimedOut,
    Network(String),
    Other(String),
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthError::InvalidClient(m) => write!(f, "Google rejected the OAuth client: {m}"),
            AuthError::InvalidGrant(m) => write!(f, "Sign-in expired or was revoked: {m}"),
            AuthError::Denied(m) => write!(f, "Sign-in was not completed: {m}"),
            AuthError::Cancelled => write!(f, "Sign-in cancelled"),
            AuthError::TimedOut => write!(f, "Sign-in timed out"),
            AuthError::Network(m) => write!(f, "Network error: {m}"),
            AuthError::Other(m) => write!(f, "{m}"),
        }
    }
}

pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

pub fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("OS random source");
    URL_SAFE_NO_PAD.encode(buf)
}

pub fn new_pkce() -> Pkce {
    // 64 random bytes → 86 base64url characters (RFC 7636 allows 43–128).
    let verifier = random_token(64);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    Pkce {
        verifier,
        challenge,
    }
}

pub fn authorize_url(client_id: &str, redirect_uri: &str, challenge: &str, state: &str) -> String {
    let mut url = url::Url::parse(AUTH_URL).expect("static URL");
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", SCOPE)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state)
        // A refresh token is only issued with offline access, and only reliably with consent.
        // select_account: always show the picker, so "Add account" can add a second account
        // instead of silently re-using the one signed in to the browser.
        .append_pair("access_type", "offline")
        .append_pair("prompt", "select_account consent");
    url.into()
}

#[derive(Debug, PartialEq, Eq)]
pub enum Callback {
    Code {
        code: String,
        state: String,
    },
    Error {
        error: String,
        state: Option<String>,
    },
    /// Not the redirect (favicon, probes): answer and keep waiting.
    Ignore,
}

/// Parses the request line of an HTTP request, e.g. `GET /?code=…&state=… HTTP/1.1`.
pub fn parse_callback(request_line: &str) -> Callback {
    let mut parts = request_line.split_whitespace();
    let (Some("GET"), Some(target)) = (parts.next(), parts.next()) else {
        return Callback::Ignore;
    };
    let Ok(url) = url::Url::parse(&format!("http://127.0.0.1{target}")) else {
        return Callback::Ignore;
    };
    if url.path() != "/" {
        return Callback::Ignore;
    }
    let get = |key: &str| {
        url.query_pairs()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.into_owned())
    };
    match (get("code"), get("error"), get("state")) {
        (_, Some(error), state) => Callback::Error { error, state },
        (Some(code), None, Some(state)) => Callback::Code { code, state },
        _ => Callback::Ignore,
    }
}

pub struct Loopback {
    listener: TcpListener,
    pub redirect_uri: String,
}

impl Loopback {
    pub fn bind() -> Result<Self, AuthError> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|e| AuthError::Other(format!("cannot listen on 127.0.0.1: {e}")))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| AuthError::Other(e.to_string()))?;
        let port = listener
            .local_addr()
            .map_err(|e| AuthError::Other(e.to_string()))?
            .port();
        Ok(Self {
            listener,
            redirect_uri: format!("http://127.0.0.1:{port}"),
        })
    }

    /// Waits for the redirect carrying `expected_state` and returns the authorization code.
    pub fn wait_for_code(
        &self,
        expected_state: &str,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<String, AuthError> {
        let deadline = Instant::now() + timeout;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(AuthError::Cancelled);
            }
            if Instant::now() >= deadline {
                return Err(AuthError::TimedOut);
            }
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if let Some(result) = handle_connection(stream, expected_state) {
                        return result;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => return Err(AuthError::Other(format!("loopback accept failed: {e}"))),
            }
        }
    }
}

/// `None` means "not the redirect, keep waiting".
fn handle_connection(
    mut stream: TcpStream,
    expected_state: &str,
) -> Option<Result<String, AuthError>> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 16 * 1024 {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let text = String::from_utf8_lossy(&buf);
    let line = text.lines().next().unwrap_or_default();
    let (result, body) = match parse_callback(line) {
        Callback::Ignore => {
            respond(&mut stream, "404 Not Found", "");
            return None;
        }
        Callback::Code { state, .. }
        | Callback::Error {
            state: Some(state), ..
        } if state != expected_state => {
            // A stale or forged redirect: refuse it but keep waiting for the real one.
            respond(
                &mut stream,
                "400 Bad Request",
                &page("This sign-in link is not the one Antarctical is waiting for."),
            );
            return None;
        }
        Callback::Code { code, .. } => (
            Ok(code),
            page("Antarctical is connected to Google Calendar. You can close this tab."),
        ),
        // Without our state an error redirect could come from anyone; ignore it.
        Callback::Error { state: None, .. } => {
            respond(
                &mut stream,
                "400 Bad Request",
                &page("This sign-in link is not the one Antarctical is waiting for."),
            );
            return None;
        }
        Callback::Error { error, .. } => (
            Err(AuthError::Denied(error.clone())),
            page(&format!(
                "Sign-in was not completed ({error}). You can close this tab."
            )),
        ),
    };
    respond(&mut stream, "200 OK", &body);
    Some(result)
}

fn page(message: &str) -> String {
    let escaped = message
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        "<!doctype html><meta charset=utf-8><title>Antarctical</title>\
         <body style=\"font:16px system-ui;background:#0d0f14;color:#f2f3f6;display:grid;place-items:center;height:90vh\">\
         <p>{escaped}</p></body>"
    )
}

fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.flush();
}

/// Never printed: `Debug` is implemented by hand without the tokens.
#[derive(Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub expires_in: u64,
    pub refresh_token: Option<String>,
}

impl fmt::Debug for TokenResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenResponse")
            .field("expires_in", &self.expires_in)
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct TokenError {
    error: String,
    error_description: Option<String>,
}

fn token_request(agent: &ureq::Agent, form: &[(&str, &str)]) -> Result<TokenResponse, AuthError> {
    let mut resp = agent
        .post(TOKEN_URL)
        .send_form(form.iter().copied())
        .map_err(|e| AuthError::Network(e.to_string()))?;
    let status = resp.status().as_u16();
    let body = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| AuthError::Network(e.to_string()))?;
    if status == 200 {
        return serde_json::from_str(&body)
            .map_err(|e| AuthError::Other(format!("unexpected token response: {e}")));
    }
    let err: TokenError = serde_json::from_str(&body).unwrap_or(TokenError {
        error: format!("HTTP {status}"),
        error_description: None,
    });
    let msg = err.error_description.unwrap_or_else(|| err.error.clone());
    Err(match err.error.as_str() {
        "invalid_client" | "unauthorized_client" => AuthError::InvalidClient(msg),
        "invalid_grant" => AuthError::InvalidGrant(msg),
        _ => AuthError::Other(msg),
    })
}

pub fn exchange_code(
    agent: &ureq::Agent,
    client: &ClientCredentials,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<TokenResponse, AuthError> {
    token_request(
        agent,
        &[
            ("code", code),
            ("client_id", &client.client_id),
            ("client_secret", &client.client_secret),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
            ("code_verifier", verifier),
        ],
    )
}

pub fn refresh(
    agent: &ureq::Agent,
    client: &ClientCredentials,
    refresh_token: &str,
) -> Result<TokenResponse, AuthError> {
    token_request(
        agent,
        &[
            ("client_id", &client.client_id),
            ("client_secret", &client.client_secret),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ],
    )
}

/// Best effort: revoking a refresh token also revokes the access tokens issued from it.
pub fn revoke(agent: &ureq::Agent, token: &str) -> bool {
    agent
        .post(REVOKE_URL)
        .send_form([("token", token)])
        .is_ok_and(|r| r.status().is_success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_s256_of_verifier() {
        let p = new_pkce();
        assert_eq!(p.verifier.len(), 86);
        assert!((43..=128).contains(&p.verifier.len()));
        let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(p.verifier.as_bytes()));
        assert_eq!(p.challenge, expected);
        // RFC 7636 appendix B test vector.
        let v = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            URL_SAFE_NO_PAD.encode(Sha256::digest(v.as_bytes())),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn authorize_url_has_required_params() {
        let u = authorize_url("id.apps", "http://127.0.0.1:5555", "chal", "st");
        let url = url::Url::parse(&u).unwrap();
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(q["client_id"], "id.apps");
        assert_eq!(q["redirect_uri"], "http://127.0.0.1:5555");
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["scope"], SCOPE);
        assert_eq!(q["access_type"], "offline");
        assert_eq!(q["prompt"], "select_account consent");
        assert_eq!(q["state"], "st");
    }

    #[test]
    fn callback_parsing() {
        assert_eq!(
            parse_callback("GET /?state=s1&code=4%2F0Ab&scope=x HTTP/1.1"),
            Callback::Code {
                code: "4/0Ab".into(),
                state: "s1".into()
            }
        );
        assert_eq!(
            parse_callback("GET /?error=access_denied&state=s1 HTTP/1.1"),
            Callback::Error {
                error: "access_denied".into(),
                state: Some("s1".into())
            }
        );
        assert_eq!(
            parse_callback("GET /favicon.ico HTTP/1.1"),
            Callback::Ignore
        );
        assert_eq!(
            parse_callback("POST /?code=a&state=b HTTP/1.1"),
            Callback::Ignore
        );
        assert_eq!(parse_callback("GET /?code=a HTTP/1.1"), Callback::Ignore);
    }

    #[test]
    fn loopback_accepts_matching_state_only() {
        let lb = Loopback::bind().unwrap();
        let addr = lb.redirect_uri.trim_start_matches("http://").to_string();
        let client = std::thread::spawn(move || {
            let send = |path: &str| {
                let mut s = TcpStream::connect(&addr).unwrap();
                write!(s, "GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
                let mut out = String::new();
                let _ = s.read_to_string(&mut out);
                out
            };
            let a = send("/favicon.ico");
            let b = send("/?code=wrong&state=other");
            let d = send("/?error=access_denied");
            let c = send("/?code=right&state=good");
            (a, b, c, d)
        });
        let cancel = AtomicBool::new(false);
        let code = lb
            .wait_for_code("good", &cancel, Duration::from_secs(10))
            .unwrap();
        assert_eq!(code, "right");
        let (a, b, c, d) = client.join().unwrap();
        assert!(a.starts_with("HTTP/1.1 404"));
        assert!(b.starts_with("HTTP/1.1 400"));
        assert!(
            d.starts_with("HTTP/1.1 400"),
            "error without state is refused"
        );
        assert!(c.starts_with("HTTP/1.1 200"));
    }

    #[test]
    fn loopback_cancel() {
        let lb = Loopback::bind().unwrap();
        let cancel = AtomicBool::new(true);
        assert_eq!(
            lb.wait_for_code("s", &cancel, Duration::from_secs(10)),
            Err(AuthError::Cancelled)
        );
    }

    #[test]
    fn debug_redacts_secret() {
        let c = ClientCredentials {
            client_id: "id".into(),
            client_secret: "hunter2".into(),
        };
        assert!(!format!("{c:?}").contains("hunter2"));
    }
}
