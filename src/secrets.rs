//! Secrets live in the desktop keyring (freedesktop Secret Service: GNOME Keyring, KWallet,
//! KeePassXC…) and nowhere else: the OAuth client id + secret entered in Settings, and the
//! Google refresh token. Access tokens are only ever held in memory.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::google::oauth::ClientCredentials;

pub const SERVICE: &str = "com.korora.Antarctical";
const CLIENT_USER: &str = "google-oauth-client";
const REFRESH_USER: &str = "google-refresh-token";

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
            self.store.delete(REFRESH_USER)?;
        }
        let raw = serde_json::to_string(&StoredClient {
            client_id: client.client_id.clone(),
            client_secret: client.client_secret.clone(),
        })
        .expect("serialize");
        self.store.set(CLIENT_USER, &raw)
    }

    pub fn clear_client(&self) -> Result<(), SecretError> {
        self.store.delete(REFRESH_USER)?;
        self.store.delete(CLIENT_USER)
    }

    pub fn refresh_token(&self) -> Result<Option<String>, SecretError> {
        self.store.get(REFRESH_USER)
    }

    pub fn set_refresh_token(&self, token: &str) -> Result<(), SecretError> {
        self.store.set(REFRESH_USER, token)
    }

    pub fn clear_refresh_token(&self) -> Result<(), SecretError> {
        self.store.delete(REFRESH_USER)
    }
}

/// The Secret Service via `keyring-core` + zbus. Items go into the default (login) collection,
/// labelled so they are recognisable in Seahorse/KWallet.
pub struct KeyringStore;

/// Connects on first use; a failure is not cached, so a keyring daemon that starts later
/// (or gets unlocked) is picked up on the next attempt.
fn ensure_default_store() -> Result<(), SecretError> {
    static READY: Mutex<bool> = Mutex::new(false);
    let mut ready = READY.lock().unwrap();
    if !*ready {
        let store = zbus_secret_service_keyring_store::Store::new()
            .map_err(|e| SecretError::Unavailable(e.to_string()))?;
        keyring_core::set_default_store(store);
        *ready = true;
    }
    Ok(())
}

fn entry(user: &str) -> Result<keyring_core::Entry, SecretError> {
    ensure_default_store()?;
    let label = match user {
        CLIENT_USER => "Antarctical: Google OAuth client",
        REFRESH_USER => "Antarctical: Google refresh token",
        _ => "Antarctical",
    };
    keyring_core::Entry::new_with_modifiers(SERVICE, user, &HashMap::from([("label", label)]))
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
            Ok(v) => Ok(Some(v)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(map_err(e)),
        }
    }

    fn set(&self, user: &str, value: &str) -> Result<(), SecretError> {
        entry(user)?.set_password(value).map_err(map_err)
    }

    fn delete(&self, user: &str) -> Result<(), SecretError> {
        match entry(user)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(e) => Err(map_err(e)),
        }
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
        s.set_refresh_token("rt").unwrap();
        // Saving the same client, or rotating its secret, keeps the session.
        s.set_client(&client("a")).unwrap();
        s.set_client(&ClientCredentials {
            client_id: "a".into(),
            client_secret: "rotated".into(),
        })
        .unwrap();
        assert_eq!(s.refresh_token().unwrap().as_deref(), Some("rt"));
        // A different client drops it.
        s.set_client(&client("b")).unwrap();
        assert_eq!(s.refresh_token().unwrap(), None);
        assert_eq!(s.client().unwrap(), Some(client("b")));
        s.set_refresh_token("rt2").unwrap();
        s.clear_client().unwrap();
        assert_eq!(s.client().unwrap(), None);
        assert_eq!(s.refresh_token().unwrap(), None);
    }

    #[test]
    fn corrupt_client_entry_is_reported() {
        let store = Arc::new(MemoryStore::default());
        store.set(CLIENT_USER, "not json").unwrap();
        let s = Secrets::new(store);
        assert!(matches!(s.client(), Err(SecretError::Corrupt(_))));
    }
}
