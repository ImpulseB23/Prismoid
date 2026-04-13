//! Persistence layer for [`TwitchTokens`].
//!
//! The [`TokenStore`] trait isolates keychain access behind a minimal
//! interface so the manager and its tests can share the same call path.
//! Prod uses [`KeychainStore`]; tests use [`MemoryStore`].
//!
//! Per ADR 37 the keychain layout is: service `prismoid.twitch`, account
//! `<broadcaster_id>`, password is a serde-JSON blob of [`TwitchTokens`].
//! One keychain prompt per save, one per load, atomic per broadcaster.

use std::collections::HashMap;
use std::sync::Mutex;

use keyring::Entry;

use super::errors::AuthError;
use super::tokens::TwitchTokens;

/// Keychain service name. Kept as a const so both the Rust side and any
/// future CLI tool / migration script read the same key.
pub const KEYCHAIN_SERVICE: &str = "prismoid.twitch";

/// Read/write/delete a persisted [`TwitchTokens`] blob, keyed by
/// broadcaster ID. Impls must be `Send + Sync` so the manager can live
/// behind an `Arc` shared with the supervisor task.
pub trait TokenStore: Send + Sync {
    fn load(&self, broadcaster_id: &str) -> Result<Option<TwitchTokens>, AuthError>;
    fn save(&self, broadcaster_id: &str, tokens: &TwitchTokens) -> Result<(), AuthError>;
    fn delete(&self, broadcaster_id: &str) -> Result<(), AuthError>;
}

/// Production [`TokenStore`] backed by the OS's native credential store
/// (Windows Credential Manager / macOS Keychain / Linux Secret Service)
/// via the `keyring` crate.
///
/// Construction does not touch the keychain; the backing store must have
/// been initialized via `keyring::set_default_store` before any method
/// on the produced `Entry` is called. The supervisor does this once at
/// process start.
#[derive(Default, Debug)]
pub struct KeychainStore;

impl TokenStore for KeychainStore {
    fn load(&self, broadcaster_id: &str) -> Result<Option<TwitchTokens>, AuthError> {
        let entry = Entry::new(KEYCHAIN_SERVICE, broadcaster_id)?;
        match entry.get_password() {
            Ok(blob) => {
                let tokens: TwitchTokens = serde_json::from_str(&blob)?;
                Ok(Some(tokens))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(AuthError::Keychain(e)),
        }
    }

    fn save(&self, broadcaster_id: &str, tokens: &TwitchTokens) -> Result<(), AuthError> {
        let entry = Entry::new(KEYCHAIN_SERVICE, broadcaster_id)?;
        let blob = serde_json::to_string(tokens)?;
        entry.set_password(&blob)?;
        Ok(())
    }

    fn delete(&self, broadcaster_id: &str) -> Result<(), AuthError> {
        let entry = Entry::new(KEYCHAIN_SERVICE, broadcaster_id)?;
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
    inner: Mutex<HashMap<String, TwitchTokens>>,
}

impl TokenStore for MemoryStore {
    fn load(&self, broadcaster_id: &str) -> Result<Option<TwitchTokens>, AuthError> {
        let guard = self.inner.lock().expect("MemoryStore mutex poisoned");
        Ok(guard.get(broadcaster_id).cloned())
    }

    fn save(&self, broadcaster_id: &str, tokens: &TwitchTokens) -> Result<(), AuthError> {
        let mut guard = self.inner.lock().expect("MemoryStore mutex poisoned");
        guard.insert(broadcaster_id.to_owned(), tokens.clone());
        Ok(())
    }

    fn delete(&self, broadcaster_id: &str) -> Result<(), AuthError> {
        let mut guard = self.inner.lock().expect("MemoryStore mutex poisoned");
        guard.remove(broadcaster_id);
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
        }
    }

    #[test]
    fn memory_store_load_missing_returns_none() {
        let store = MemoryStore::default();
        assert!(store.load("unknown").unwrap().is_none());
    }

    #[test]
    fn memory_store_save_then_load_returns_same() {
        let store = MemoryStore::default();
        let t = sample();
        store.save("b1", &t).unwrap();
        assert_eq!(store.load("b1").unwrap().unwrap(), t);
    }

    #[test]
    fn memory_store_save_overwrites() {
        let store = MemoryStore::default();
        let mut t = sample();
        store.save("b1", &t).unwrap();
        t.access_token = "at2".into();
        store.save("b1", &t).unwrap();
        assert_eq!(store.load("b1").unwrap().unwrap(), t);
    }

    #[test]
    fn memory_store_delete_removes_entry() {
        let store = MemoryStore::default();
        store.save("b1", &sample()).unwrap();
        store.delete("b1").unwrap();
        assert!(store.load("b1").unwrap().is_none());
    }

    #[test]
    fn memory_store_delete_missing_is_noop() {
        let store = MemoryStore::default();
        store.delete("never-existed").unwrap();
    }

    #[test]
    fn memory_store_broadcasters_are_isolated() {
        let store = MemoryStore::default();
        let mut a = sample();
        a.access_token = "A".into();
        let mut b = sample();
        b.access_token = "B".into();
        store.save("alpha", &a).unwrap();
        store.save("beta", &b).unwrap();
        assert_eq!(store.load("alpha").unwrap().unwrap().access_token, "A");
        assert_eq!(store.load("beta").unwrap().unwrap().access_token, "B");
    }
}
