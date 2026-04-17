//! Auth state and serializable view types shared by the Twitch sign-in
//! commands and the sidecar supervisor. Keeping the business logic out
//! of the `#[tauri::command]` adapters in `commands.rs` lets us test
//! every branch without spinning up a Tauri runtime / WebView2.

use std::sync::Arc;

use serde::Serialize;
use tokio::sync::{Mutex, Notify};

use super::errors::AuthError;
use super::manager::{AuthManager, PendingDeviceFlow};

/// Shared auth state held in Tauri's managed-state map. The supervisor
/// holds a clone of the same `Arc<AuthManager>` and the same
/// `Arc<Notify>` so a successful sign-in immediately wakes the
/// supervisor's `waiting_for_auth` sleep instead of forcing the user
/// to wait up to 30 s for the next poll tick.
pub struct AuthState {
    pub manager: Arc<AuthManager>,
    pub pending: Mutex<Option<PendingDeviceFlow>>,
    /// Notifier the supervisor awaits while idle in `waiting_for_auth`.
    /// Successful login + logout both fire it: login so a fresh sidecar
    /// spins up immediately; logout so any in-progress sidecar tears
    /// down on the next loop iteration.
    pub wakeup: Arc<Notify>,
}

impl AuthState {
    pub fn new(manager: Arc<AuthManager>, wakeup: Arc<Notify>) -> Self {
        Self {
            manager,
            pending: Mutex::new(None),
            wakeup,
        }
    }

    pub fn status(&self) -> Result<AuthStatus, AuthCommandError> {
        let login = self.manager.peek_login()?;
        Ok(match login {
            Some(l) => AuthStatus {
                state: AuthStatusState::LoggedIn,
                login: Some(l),
            },
            None => AuthStatus {
                state: AuthStatusState::LoggedOut,
                login: None,
            },
        })
    }

    pub async fn start_login(&self) -> Result<DeviceCodeView, AuthCommandError> {
        let pending = self.manager.start_device_flow().await?;
        let view = DeviceCodeView {
            verification_uri: pending.details().verification_uri.clone(),
            user_code: pending.details().user_code.clone(),
            expires_in_secs: pending.details().expires_in,
        };
        *self.pending.lock().await = Some(pending);
        Ok(view)
    }

    pub async fn complete_login(&self) -> Result<AuthStatus, AuthCommandError> {
        let pending = self.pending.lock().await.take().ok_or(AuthCommandError {
            kind: "no_pending_flow",
            message: "twitch_start_login has not been called".into(),
        })?;
        let tokens = self.manager.complete_device_flow(pending).await?;
        // notify_one stores a permit if the supervisor isn't currently
        // parked on notified(), so the wake can't be lost between login
        // completing and the supervisor reaching its await.
        self.wakeup.notify_one();
        Ok(AuthStatus {
            state: AuthStatusState::LoggedIn,
            login: Some(tokens.login),
        })
    }

    pub async fn cancel_login(&self) {
        self.pending.lock().await.take();
    }

    pub async fn logout(&self) -> Result<(), AuthCommandError> {
        self.manager.logout()?;
        self.pending.lock().await.take();
        self.wakeup.notify_one();
        Ok(())
    }
}

/// Result of `twitch_auth_status`. The frontend uses `state` to pick
/// between the SignIn overlay and the chat view; `login` is rendered
/// in the header bar when logged in.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AuthStatus {
    pub state: AuthStatusState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthStatusState {
    LoggedOut,
    LoggedIn,
}

/// Shape of the device-code details surfaced to the frontend. We
/// deliberately don't expose `device_code` (it's an exchange-only
/// secret) or `interval` (the manager handles polling cadence
/// internally via `wait_for_code`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DeviceCodeView {
    pub verification_uri: String,
    pub user_code: String,
    pub expires_in_secs: u64,
}

/// Frontend-facing error. `AuthError` itself isn't `Serialize` (it
/// carries `keyring::Error` / `serde_json::Error`), so we map to a
/// stable tag the UI can switch on plus a human message for diagnostics.
#[derive(Debug, Clone, Serialize)]
pub struct AuthCommandError {
    pub kind: &'static str,
    pub message: String,
}

impl From<AuthError> for AuthCommandError {
    fn from(err: AuthError) -> Self {
        let kind = match &err {
            AuthError::NoTokens => "no_tokens",
            AuthError::RefreshTokenInvalid => "refresh_invalid",
            AuthError::DeviceCodeExpired => "device_code_expired",
            AuthError::UserDenied => "user_denied",
            AuthError::Keychain(_) => "keychain",
            AuthError::OAuth(_) => "oauth",
            AuthError::Json(_) => "json",
            AuthError::Config(_) => "config",
        };
        Self {
            kind,
            message: err.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::twitch_auth::storage::{MemoryStore, TokenStore};
    use crate::twitch_auth::tokens::TwitchTokens;
    use crate::twitch_auth::AuthManagerBuilder;

    fn http_client() -> reqwest::Client {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("reqwest client")
    }

    fn fixture_tokens() -> TwitchTokens {
        TwitchTokens {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            expires_at_ms: i64::MAX,
            scopes: vec!["user:read:chat".into()],
            user_id: "12345".into(),
            login: "tester".into(),
        }
    }

    fn build_state_with_store(store: MemoryStore) -> AuthState {
        let manager =
            Arc::new(AuthManagerBuilder::new("test_client_id").build(store, http_client()));
        AuthState::new(manager, Arc::new(Notify::new()))
    }

    #[test]
    fn auth_command_error_maps_no_tokens() {
        let mapped: AuthCommandError = AuthError::NoTokens.into();
        assert_eq!(mapped.kind, "no_tokens");
    }

    #[test]
    fn auth_command_error_maps_refresh_invalid() {
        let mapped: AuthCommandError = AuthError::RefreshTokenInvalid.into();
        assert_eq!(mapped.kind, "refresh_invalid");
    }

    #[test]
    fn auth_command_error_maps_device_code_expired() {
        let mapped: AuthCommandError = AuthError::DeviceCodeExpired.into();
        assert_eq!(mapped.kind, "device_code_expired");
    }

    #[test]
    fn auth_command_error_maps_user_denied() {
        let mapped: AuthCommandError = AuthError::UserDenied.into();
        assert_eq!(mapped.kind, "user_denied");
    }

    #[test]
    fn auth_command_error_maps_keychain() {
        let mapped: AuthCommandError = AuthError::Keychain(keyring::Error::NoEntry).into();
        assert_eq!(mapped.kind, "keychain");
        assert!(!mapped.message.is_empty());
    }

    #[test]
    fn auth_command_error_maps_json() {
        let json_err = serde_json::from_str::<TwitchTokens>("not json").unwrap_err();
        let mapped: AuthCommandError = AuthError::Json(json_err).into();
        assert_eq!(mapped.kind, "json");
    }

    #[test]
    fn auth_command_error_maps_config() {
        let mapped: AuthCommandError = AuthError::Config("bad scope".into()).into();
        assert_eq!(mapped.kind, "config");
        assert!(mapped.message.contains("bad scope"));
    }

    #[test]
    fn auth_status_serializes_logged_out_without_login_field() {
        let s = AuthStatus {
            state: AuthStatusState::LoggedOut,
            login: None,
        };
        let v: serde_json::Value = serde_json::to_value(&s).unwrap();
        assert_eq!(v["state"], "logged_out");
        assert!(
            v.get("login").is_none(),
            "logged_out status should not include login key, got {v}"
        );
    }

    #[test]
    fn auth_status_serializes_logged_in_with_login() {
        let s = AuthStatus {
            state: AuthStatusState::LoggedIn,
            login: Some("tester".into()),
        };
        let v: serde_json::Value = serde_json::to_value(&s).unwrap();
        assert_eq!(v["state"], "logged_in");
        assert_eq!(v["login"], "tester");
    }

    #[tokio::test]
    async fn status_returns_logged_in_when_tokens_present() {
        let store = MemoryStore::default();
        store.save(&fixture_tokens()).unwrap();
        let state = build_state_with_store(store);

        let status = state.status().unwrap();
        assert_eq!(status.state, AuthStatusState::LoggedIn);
        assert_eq!(status.login.as_deref(), Some("tester"));
    }

    #[tokio::test]
    async fn status_returns_logged_out_when_no_tokens() {
        let store = MemoryStore::default();
        let state = build_state_with_store(store);

        let status = state.status().unwrap();
        assert_eq!(status.state, AuthStatusState::LoggedOut);
        assert!(status.login.is_none());
    }

    #[tokio::test]
    async fn complete_login_without_pending_returns_no_pending_flow() {
        let state = build_state_with_store(MemoryStore::default());
        let err = state.complete_login().await.unwrap_err();
        assert_eq!(err.kind, "no_pending_flow");
    }

    #[tokio::test]
    async fn logout_when_empty_store_still_clears_and_notifies() {
        let state = build_state_with_store(MemoryStore::default());
        let wakeup = state.wakeup.clone();

        state.logout().await.unwrap();
        // notify_one stored a permit, so a subsequent notified() resolves
        // immediately even though no waiter was parked at signal time.
        tokio::time::timeout(std::time::Duration::from_secs(1), wakeup.notified())
            .await
            .expect("permit should be available");
    }

    #[tokio::test]
    async fn cancel_login_is_idempotent() {
        let state = build_state_with_store(MemoryStore::default());
        state.cancel_login().await;
        state.cancel_login().await;
        assert!(state.pending.lock().await.is_none());
    }

    #[tokio::test]
    async fn logout_wipes_store_and_pending_and_notifies() {
        let store = MemoryStore::default();
        store.save(&fixture_tokens()).unwrap();
        let state = build_state_with_store(store);
        let wakeup = state.wakeup.clone();
        let waiter = tokio::spawn(async move {
            wakeup.notified().await;
        });

        state.logout().await.unwrap();

        assert!(state.manager.peek_login().unwrap().is_none());
        tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .expect("waiter should be woken")
            .expect("waiter task panicked");
    }

    #[tokio::test]
    async fn cancel_login_clears_pending_slot() {
        let state = build_state_with_store(MemoryStore::default());
        state.cancel_login().await;
        assert!(state.pending.lock().await.is_none());
    }
}
