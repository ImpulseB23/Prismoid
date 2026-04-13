//! Persistence layer for [`TwitchTokens`].
//!
//! Single-account per ADR 30: one blob per app under a fixed
//! `(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)` pair. The `TokenStore` trait
//! isolates keychain access behind a minimal interface so the manager
//! and its tests can share the same call path.
//!
//! A future "multi-account" feature would keep this trait's shape and
//! add a sibling `MultiTokenStore` keyed by broadcaster_id. Today
//! one account is all we need.

use std::sync::Mutex;

use keyring::Entry;

use super::errors::AuthError;
use super::tokens::TwitchTokens;

/// Keychain service name. All Prismoid-Twitch entries live under this.
pub const KEYCHAIN_SERVICE: &str = "prismoid.twitch";

/// Keychain account name for the single-account slot. ADR 30 pins
/// one-account-per-platform in v1. The entry's stored blob carries the
/// actual user_id/login inside it (see [`TwitchTokens`]).
pub const KEYCHAIN_ACCOUNT: &str = "active";

/// Read/write/delete the persisted [`TwitchTokens`] for the single
/// active Twitch account. Impls must be `Send + Sync` so the manager
/// can live behind an `Arc` shared with the supervisor task.
pub trait TokenStore: Send + Sync {
    fn load(&self) -> Result<Option<TwitchTokens>, AuthError>;
    fn save(&self, tokens: &TwitchTokens) -> Result<(), AuthError>;
    fn delete(&self) -> Result<(), AuthError>;
}

/// Production [`TokenStore`] backed by the OS's native credential store
/// (Windows Credential Manager / macOS Keychain / Linux Secret Service)
/// via the `keyring` crate.
#[derive(Default, Debug)]
pub struct KeychainStore;

impl TokenStore for KeychainStore {
    fn load(&self) -> Result<Option<TwitchTokens>, AuthError> {
        let entry = Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)?;
        match entry.get_password() {
            Ok(blob) => {
                let tokens: TwitchTokens = serde_json::from_str(&blob)?;
                Ok(Some(tokens))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(AuthError::Keychain(e)),
        }
    }

    fn save(&self, tokens: &TwitchTokens) -> Result<(), AuthError> {
        let entry = Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)?;
        let blob = serde_json::to_string(tokens)?;
        entry.set_password(&blob)?;
        Ok(())
    }

    fn delete(&self) -> Result<(), AuthError> {
        let entry = Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(AuthError::Keychain(e)),
        }
    }
}

/// In-memory [`TokenStore`] for tests. Never touches the OS keychain so
/// the test suite doesn't pop credential prompts, doesn't leak fixtures
/// across runs, and doesn't conflict with any real user's saved tokens.
#[derive(Default, Debug)]
pub struct MemoryStore {
    inner: Mutex<Option<TwitchTokens>>,
}

impl TokenStore for MemoryStore {
    fn load(&self) -> Result<Option<TwitchTokens>, AuthError> {
        let guard = self.inner.lock().expect("MemoryStore mutex poisoned");
        Ok(guard.clone())
    }

    fn save(&self, tokens: &TwitchTokens) -> Result<(), AuthError> {
        let mut guard = self.inner.lock().expect("MemoryStore mutex poisoned");
        *guard = Some(tokens.clone());
        Ok(())
    }

    fn delete(&self) -> Result<(), AuthError> {
        let mut guard = self.inner.lock().expect("MemoryStore mutex poisoned");
        *guard = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> TwitchTokens {
        TwitchTokens {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            expires_at_ms: 1_000_000,
            scopes: vec!["user:read:chat".into()],
            user_id: "570722168".into(),
            login: "impulseb23".into(),
        }
    }

    #[test]
    fn memory_store_load_missing_returns_none() {
        let store = MemoryStore::default();
        assert!(store.load().unwrap().is_none());
    }

    #[test]
    fn memory_store_save_then_load_returns_same() {
        let store = MemoryStore::default();
        let t = sample();
        store.save(&t).unwrap();
        assert_eq!(store.load().unwrap().unwrap(), t);
    }

    #[test]
    fn memory_store_save_overwrites() {
        let store = MemoryStore::default();
        let mut t = sample();
        store.save(&t).unwrap();
        t.access_token = "at2".into();
        store.save(&t).unwrap();
        assert_eq!(store.load().unwrap().unwrap(), t);
    }

    #[test]
    fn memory_store_delete_removes_entry() {
        let store = MemoryStore::default();
        store.save(&sample()).unwrap();
        store.delete().unwrap();
        assert!(store.load().unwrap().is_none());
    }

    #[test]
    fn memory_store_delete_missing_is_noop() {
        let store = MemoryStore::default();
        store.delete().unwrap();
    }
}
