//! Persistence layer for [`YouTubeTokens`].
//!
//! Single-account per ADR 30: one blob per app under a fixed
//! `(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)` pair. Mirrors
//! `twitch_auth::storage` shape; the only difference is the service
//! name (`prismoid.youtube` instead of `prismoid.twitch`) so the two
//! providers' entries don't collide in the OS credential store.

use std::sync::Mutex;

use keyring::Entry;

use super::errors::AuthError;
use super::tokens::YouTubeTokens;

pub const KEYCHAIN_SERVICE: &str = "prismoid.youtube";
pub const KEYCHAIN_ACCOUNT: &str = "active";

pub trait TokenStore: Send + Sync {
    fn load(&self) -> Result<Option<YouTubeTokens>, AuthError>;
    fn save(&self, tokens: &YouTubeTokens) -> Result<(), AuthError>;
    fn delete(&self) -> Result<(), AuthError>;
}

#[derive(Default, Debug)]
pub struct KeychainStore;

impl TokenStore for KeychainStore {
    fn load(&self) -> Result<Option<YouTubeTokens>, AuthError> {
        let entry = Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)?;
        match entry.get_password() {
            Ok(blob) => {
                let tokens: YouTubeTokens = serde_json::from_str(&blob)?;
                Ok(Some(tokens))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(AuthError::Keychain(e)),
        }
    }

    fn save(&self, tokens: &YouTubeTokens) -> Result<(), AuthError> {
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

#[derive(Default, Debug)]
pub struct MemoryStore {
    inner: Mutex<Option<YouTubeTokens>>,
}

impl TokenStore for MemoryStore {
    fn load(&self) -> Result<Option<YouTubeTokens>, AuthError> {
        let guard = self.inner.lock().expect("MemoryStore mutex poisoned");
        Ok(guard.clone())
    }

    fn save(&self, tokens: &YouTubeTokens) -> Result<(), AuthError> {
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

    fn sample() -> YouTubeTokens {
        YouTubeTokens {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            expires_at_ms: 1_000_000,
            scopes: vec!["https://www.googleapis.com/auth/youtube.readonly".into()],
            channel_id: "UC123".into(),
            channel_title: "Test".into(),
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

    #[test]
    fn keychain_service_is_distinct_from_twitch() {
        // Sanity: if these collide a single keychain entry is shared
        // and the two providers stomp each other's tokens.
        assert_ne!(KEYCHAIN_SERVICE, crate::twitch_auth::KEYCHAIN_SERVICE);
    }
}
