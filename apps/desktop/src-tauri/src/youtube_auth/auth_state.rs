//! Auth state and serializable view types shared by the YouTube
//! sign-in commands and the (future) sidecar supervisor. Mirrors the
//! shape of `twitch_auth::auth_state` so the frontend can use a
//! symmetric `<Provider>SignIn` component layout.

use std::sync::Arc;

use serde::Serialize;
use tokio::sync::{Mutex, Notify};

use super::errors::AuthError;
use super::manager::{AuthManager, PendingLogin};

pub struct AuthState {
    pub manager: Arc<AuthManager>,
    pub pending: Mutex<Option<PendingLogin>>,
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
        let title = self.manager.peek_channel_title()?;
        Ok(match title {
            Some(t) => AuthStatus {
                state: AuthStatusState::LoggedIn,
                channel_title: Some(t),
            },
            None => AuthStatus {
                state: AuthStatusState::LoggedOut,
                channel_title: None,
            },
        })
    }

    pub async fn start_login(&self) -> Result<PkceFlowView, AuthCommandError> {
        let pending = self.manager.start_login().await?;
        let view = PkceFlowView {
            authorization_uri: pending.authorization_uri().to_string(),
        };
        *self.pending.lock().await = Some(pending);
        Ok(view)
    }

    pub async fn complete_login(&self) -> Result<AuthStatus, AuthCommandError> {
        let pending = self.pending.lock().await.take().ok_or(AuthCommandError {
            kind: "no_pending_flow",
            message: "youtube_start_login has not been called".into(),
        })?;
        let tokens = self.manager.complete_login(pending).await?;
        self.wakeup.notify_one();
        Ok(AuthStatus {
            state: AuthStatusState::LoggedIn,
            channel_title: Some(tokens.channel_title),
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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AuthStatus {
    pub state: AuthStatusState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_title: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthStatusState {
    LoggedOut,
    LoggedIn,
}

/// Shape surfaced to the frontend after `start_login`. The frontend's
/// only job is to open `authorization_uri` in the system browser and
/// then call `complete_login`, which blocks on the loopback redirect.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PkceFlowView {
    pub authorization_uri: String,
}

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
            AuthError::UserDenied => "user_denied",
            AuthError::LoopbackBind(_) => "loopback_bind",
            AuthError::StateMismatch => "state_mismatch",
            AuthError::Keychain(_) => "keychain",
            AuthError::OAuth(_) => "oauth",
            AuthError::Json(_) => "json",
            AuthError::NoChannel => "no_channel",
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
    use crate::youtube_auth::storage::{MemoryStore, TokenStore};
    use crate::youtube_auth::tokens::YouTubeTokens;
    use crate::youtube_auth::AuthManagerBuilder;

    fn http_client() -> reqwest::Client {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("reqwest client")
    }

    fn fixture_tokens() -> YouTubeTokens {
        YouTubeTokens {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            expires_at_ms: i64::MAX,
            scopes: vec!["scope-a".into()],
            channel_id: "UC123".into(),
            channel_title: "Test Channel".into(),
        }
    }

    fn build_state_with_store(store: MemoryStore) -> AuthState {
        let manager = Arc::new(AuthManagerBuilder::new("c", "s").build(store, http_client()));
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
    fn auth_command_error_maps_user_denied() {
        let mapped: AuthCommandError = AuthError::UserDenied.into();
        assert_eq!(mapped.kind, "user_denied");
    }

    #[test]
    fn auth_command_error_maps_loopback_bind() {
        let mapped: AuthCommandError = AuthError::LoopbackBind("addr in use".into()).into();
        assert_eq!(mapped.kind, "loopback_bind");
        assert!(mapped.message.contains("addr in use"));
    }

    #[test]
    fn auth_command_error_maps_state_mismatch() {
        let mapped: AuthCommandError = AuthError::StateMismatch.into();
        assert_eq!(mapped.kind, "state_mismatch");
    }

    #[test]
    fn auth_command_error_maps_no_channel() {
        let mapped: AuthCommandError = AuthError::NoChannel.into();
        assert_eq!(mapped.kind, "no_channel");
    }

    #[test]
    fn auth_status_serializes_logged_out_without_channel() {
        let s = AuthStatus {
            state: AuthStatusState::LoggedOut,
            channel_title: None,
        };
        let v: serde_json::Value = serde_json::to_value(&s).unwrap();
        assert_eq!(v["state"], "logged_out");
        assert!(v.get("channel_title").is_none());
    }

    #[test]
    fn auth_status_serializes_logged_in_with_channel() {
        let s = AuthStatus {
            state: AuthStatusState::LoggedIn,
            channel_title: Some("Test".into()),
        };
        let v: serde_json::Value = serde_json::to_value(&s).unwrap();
        assert_eq!(v["state"], "logged_in");
        assert_eq!(v["channel_title"], "Test");
    }

    #[tokio::test]
    async fn status_returns_logged_in_when_tokens_present() {
        let store = MemoryStore::default();
        store.save(&fixture_tokens()).unwrap();
        let state = build_state_with_store(store);
        let status = state.status().unwrap();
        assert_eq!(status.state, AuthStatusState::LoggedIn);
        assert_eq!(status.channel_title.as_deref(), Some("Test Channel"));
    }

    #[tokio::test]
    async fn status_returns_logged_out_when_no_tokens() {
        let state = build_state_with_store(MemoryStore::default());
        let status = state.status().unwrap();
        assert_eq!(status.state, AuthStatusState::LoggedOut);
        assert!(status.channel_title.is_none());
    }

    #[tokio::test]
    async fn complete_login_without_pending_returns_no_pending_flow() {
        let state = build_state_with_store(MemoryStore::default());
        let err = state.complete_login().await.unwrap_err();
        assert_eq!(err.kind, "no_pending_flow");
    }

    #[tokio::test]
    async fn cancel_login_is_idempotent() {
        let state = build_state_with_store(MemoryStore::default());
        state.cancel_login().await;
        state.cancel_login().await;
        assert!(state.pending.lock().await.is_none());
    }

    #[tokio::test]
    async fn logout_when_empty_store_still_clears_and_notifies() {
        let state = build_state_with_store(MemoryStore::default());
        let wakeup = state.wakeup.clone();
        state.logout().await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), wakeup.notified())
            .await
            .expect("permit should be available");
    }

    #[tokio::test]
    async fn logout_wipes_store_and_pending_and_notifies() {
        let store = MemoryStore::default();
        store.save(&fixture_tokens()).unwrap();
        let state = build_state_with_store(store);
        let wakeup = state.wakeup.clone();
        let waiter = tokio::spawn(async move { wakeup.notified().await });

        state.logout().await.unwrap();

        assert!(state.manager.peek_channel_title().unwrap().is_none());
        tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .expect("waiter should be woken")
            .expect("waiter task panicked");
    }

    #[tokio::test]
    async fn start_login_stashes_pending_and_returns_authorization_uri() {
        let state = build_state_with_store(MemoryStore::default());
        let view = state.start_login().await.unwrap();
        assert!(view
            .authorization_uri
            .starts_with("https://accounts.google.com/"));
        assert!(view
            .authorization_uri
            .contains("code_challenge_method=S256"));
        assert!(view
            .authorization_uri
            .contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A"));
        assert!(state.pending.lock().await.is_some());
        // Drop the loopback listener so the test doesn't leak the port.
        state.cancel_login().await;
    }
}
