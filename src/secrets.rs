//! Secrets live in the desktop keyring (freedesktop Secret Service: GNOME Keyring, KWallet,
//! KeePassXC…) and nowhere else: the OAuth client id + secret entered in Settings, and one
//! refresh token per Google account. Access tokens are only ever held in memory.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::google::oauth::ClientCredentials;

pub const SERVICE: &str = "korora.tech.Antarctical";
/// The service name items were stored under before; each one moves to [`SERVICE`] when first read.
const LEGACY_SERVICE: &str = "com.korora.Antarctical";
const CLIENT_USER: &str = "google-oauth-client";
/// JSON map of account email → refresh token, in one item: one unlock, one atomic write.
const TOKENS_USER: &str = "google-refresh-tokens";
/// The single-account item from before multi-account support; migrated on first start.
const LEGACY_REFRESH_USER: &str = "google-refresh-token";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretError {
    /// No Secret Service on the session bus, or it refused access (locked, denied).
    Unavailable(String),
    Corrupt(String),
}

impl std::fmt::Display for SecretError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SecretError::Unavailable(m) => write!(f, "Keyring unavailable: {m}"),
            SecretError::Corrupt(m) => write!(f, "Keyring entry is unreadable: {m}"),
        }
    }
}

pub trait SecretStore: Send + Sync {
    fn get(&self, user: &str) -> Result<Option<String>, SecretError>;
    fn set(&self, user: &str, value: &str) -> Result<(), SecretError>;
    fn delete(&self, user: &str) -> Result<(), SecretError>;
}

#[derive(Serialize, Deserialize)]
struct StoredClient {
    client_id: String,
    client_secret: String,
}

/// Typed access to Antarctical's entries on top of any [`SecretStore`].
#[derive(Clone)]
pub struct Secrets {
    store: Arc<dyn SecretStore>,
}

impl Secrets {
    pub fn new(store: Arc<dyn SecretStore>) -> Self {
        Self { store }
    }

    pub fn keyring() -> Self {
        Self::new(Arc::new(KeyringStore))
    }

    #[cfg(test)]
    pub fn memory() -> Self {
        Self::new(Arc::new(MemoryStore::default()))
    }

    pub fn client(&self) -> Result<Option<ClientCredentials>, SecretError> {
        let Some(raw) = self.store.get(CLIENT_USER)? else {
            return Ok(None);
        };
        let c: StoredClient =
            serde_json::from_str(&raw).map_err(|e| SecretError::Corrupt(e.to_string()))?;
        Ok(Some(ClientCredentials {
            client_id: c.client_id,
            client_secret: c.client_secret,
        }))
    }

    /// Stores the client. A different client id invalidates the refresh token (Google ties
    /// refresh tokens to the client that obtained them), so it is removed first: a failure
    /// can then never leave a new client paired with an old token.
    pub fn set_client(&self, client: &ClientCredentials) -> Result<(), SecretError> {
        let previous = self.client().ok().flatten();
        if previous.as_ref().map(|c| &c.client_id) != Some(&client.client_id) {
            self.clear_tokens()?;
        }
        let raw = serde_json::to_string(&StoredClient {
            client_id: client.client_id.clone(),
            client_secret: client.client_secret.clone(),
        })
        .expect("StoredClient contains only JSON-serializable strings");
        self.store.set(CLIENT_USER, &raw)
    }

    pub fn clear_client(&self) -> Result<(), SecretError> {
        self.clear_tokens()?;
        self.store.delete(CLIENT_USER)
    }

    /// Refresh tokens by account email.
    pub fn tokens(&self) -> Result<BTreeMap<String, String>, SecretError> {
        match self.store.get(TOKENS_USER)? {
            None => Ok(BTreeMap::new()),
            Some(raw) => {
                serde_json::from_str(&raw).map_err(|e| SecretError::Corrupt(e.to_string()))
            }
        }
    }

    fn write_tokens(&self, tokens: &BTreeMap<String, String>) -> Result<(), SecretError> {
        if tokens.is_empty() {
            self.store.delete(TOKENS_USER)
        } else {
            self.store.set(
                TOKENS_USER,
                &serde_json::to_string(tokens)
                    .expect("refresh-token maps contain only JSON-serializable strings"),
            )
        }
    }

    pub fn set_token(&self, account: &str, token: &str) -> Result<(), SecretError> {
        let mut tokens = self.tokens()?;
        tokens.insert(account.to_string(), token.to_string());
        self.write_tokens(&tokens)
    }

    pub fn remove_token(&self, account: &str) -> Result<(), SecretError> {
        let mut tokens = self.tokens()?;
        if tokens.remove(account).is_some() {
            self.write_tokens(&tokens)?;
        }
        Ok(())
    }

    /// Forgets every account (including a not yet migrated single-account token).
    pub fn clear_tokens(&self) -> Result<(), SecretError> {
        self.store.delete(LEGACY_REFRESH_USER)?;
        self.store.delete(TOKENS_USER)
    }

    /// A refresh token stored by a single-account build, whose account is not known yet.
    pub fn legacy_token(&self) -> Result<Option<String>, SecretError> {
        self.store.get(LEGACY_REFRESH_USER)
    }

    pub fn delete_legacy_token(&self) -> Result<(), SecretError> {
        self.store.delete(LEGACY_REFRESH_USER)
    }
}

/// The Secret Service via `keyring-core` + zbus. Items go into the default (login) collection,
/// labelled so they are recognisable in Seahorse/KWallet.
pub struct KeyringStore;

/// Connects on first use; a failure is not cached, so a keyring daemon that starts later
/// (or gets unlocked) is picked up on the next attempt.
fn ensure_default_store() -> Result<(), SecretError> {
    static READY: Mutex<bool> = Mutex::new(false);
    let mut ready = READY
        .lock()
        .map_err(|_| SecretError::Unavailable("keyring initialization lock was poisoned".into()))?;
    if !*ready {
        let store = zbus_secret_service_keyring_store::Store::new()
            .map_err(|e| SecretError::Unavailable(e.to_string()))?;
        keyring_core::set_default_store(store);
        *ready = true;
    }
    Ok(())
}

fn entry(user: &str) -> Result<keyring_core::Entry, SecretError> {
    entry_in(SERVICE, user)
}

fn entry_in(service: &str, user: &str) -> Result<keyring_core::Entry, SecretError> {
    ensure_default_store()?;
    let label = match user {
        CLIENT_USER => "Antarctical: Google OAuth client",
        TOKENS_USER => "Antarctical: Google refresh tokens",
        LEGACY_REFRESH_USER => "Antarctical: Google refresh token",
        _ => "Antarctical",
    };
    keyring_core::Entry::new_with_modifiers(service, user, &HashMap::from([("label", label)]))
        .map_err(map_err)
}

fn map_err(e: keyring_core::Error) -> SecretError {
    match e {
        keyring_core::Error::BadEncoding(_) | keyring_core::Error::BadDataFormat(..) => {
            SecretError::Corrupt(e.to_string())
        }
        other => SecretError::Unavailable(other.to_string()),
    }
}

impl SecretStore for KeyringStore {
    fn get(&self, user: &str) -> Result<Option<String>, SecretError> {
        match entry(user)?.get_password() {
            Ok(v) => return Ok(Some(v)),
            Err(keyring_core::Error::NoEntry) => {}
            Err(e) => return Err(map_err(e)),
        }
        // Copied before the old item is deleted, so a failure part-way loses nothing.
        let legacy = entry_in(LEGACY_SERVICE, user)?;
        match legacy.get_password() {
            Ok(v) => {
                self.set(user, &v)?;
                legacy.delete_credential().map_err(map_err)?;
                Ok(Some(v))
            }
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(map_err(e)),
        }
    }

    fn set(&self, user: &str, value: &str) -> Result<(), SecretError> {
        entry(user)?.set_password(value).map_err(map_err)
    }

    /// Deletes the item under the old service name too, so a removal cannot be undone by a
    /// later migration.
    fn delete(&self, user: &str) -> Result<(), SecretError> {
        for service in [SERVICE, LEGACY_SERVICE] {
            match entry_in(service, user)?.delete_credential() {
                Ok(()) | Err(keyring_core::Error::NoEntry) => {}
                Err(e) => return Err(map_err(e)),
            }
        }
        Ok(())
    }
}

/// Process-local store for tests.
#[cfg(test)]
#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);

#[cfg(test)]
impl SecretStore for MemoryStore {
    fn get(&self, user: &str) -> Result<Option<String>, SecretError> {
        Ok(self.0.lock().unwrap().get(user).cloned())
    }
    fn set(&self, user: &str, value: &str) -> Result<(), SecretError> {
        self.0.lock().unwrap().insert(user.into(), value.into());
        Ok(())
    }
    fn delete(&self, user: &str) -> Result<(), SecretError> {
        self.0.lock().unwrap().remove(user);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(id: &str) -> ClientCredentials {
        ClientCredentials {
            client_id: id.into(),
            client_secret: "s".into(),
        }
    }

    #[test]
    fn client_roundtrip_and_token_invalidation() {
        let s = Secrets::memory();
        assert_eq!(s.client().unwrap(), None);
        s.set_client(&client("a")).unwrap();
        s.set_token("me@x", "rt").unwrap();
        // Saving the same client, or rotating its secret, keeps the sessions.
        s.set_client(&client("a")).unwrap();
        s.set_client(&ClientCredentials {
            client_id: "a".into(),
            client_secret: "rotated".into(),
        })
        .unwrap();
        assert_eq!(
            s.tokens().unwrap().get("me@x").map(String::as_str),
            Some("rt")
        );
        // A different client drops every account.
        s.set_client(&client("b")).unwrap();
        assert!(s.tokens().unwrap().is_empty());
        assert_eq!(s.client().unwrap(), Some(client("b")));
        s.set_token("me@x", "rt2").unwrap();
        s.clear_client().unwrap();
        assert_eq!(s.client().unwrap(), None);
        assert!(s.tokens().unwrap().is_empty());
    }

    #[test]
    fn tokens_per_account() {
        let s = Secrets::memory();
        s.set_token("a@x", "1").unwrap();
        s.set_token("b@x", "2").unwrap();
        s.set_token("a@x", "3").unwrap();
        let t = s.tokens().unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(t["a@x"], "3");
        s.remove_token("a@x").unwrap();
        assert_eq!(s.tokens().unwrap().keys().collect::<Vec<_>>(), vec!["b@x"]);
        s.remove_token("b@x").unwrap();
        assert!(s.tokens().unwrap().is_empty());
    }

    #[test]
    fn legacy_token_is_cleared_with_the_accounts() {
        let store = Arc::new(MemoryStore::default());
        store.set(LEGACY_REFRESH_USER, "old").unwrap();
        let s = Secrets::new(store);
        assert_eq!(s.legacy_token().unwrap().as_deref(), Some("old"));
        s.clear_tokens().unwrap();
        assert_eq!(s.legacy_token().unwrap(), None);
    }

    #[test]
    fn corrupt_client_entry_is_reported() {
        let store = Arc::new(MemoryStore::default());
        store.set(CLIENT_USER, "not json").unwrap();
        let s = Secrets::new(store);
        assert!(matches!(s.client(), Err(SecretError::Corrupt(_))));
    }
}
