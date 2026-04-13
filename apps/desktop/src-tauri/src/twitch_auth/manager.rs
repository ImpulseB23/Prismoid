//! `AuthManager` — the façade callers interact with.
//!
//! Wraps the `twitch_oauth2` crate's Twitch-aware OAuth types + a
//! [`TokenStore`] for keychain persistence. ADR 37 (DCF public client)
//! and ADR 29 (proactive 5-min refresh) are enforced here.
//!
//! Single-account per ADR 30: `load_or_refresh` / `complete_device_flow`
//! take no broadcaster_id argument — the store holds exactly one entry.
//! The user_id + login land on the returned `TwitchTokens` (sourced from
//! the DCF response's `UserToken.user_id` / `UserToken.login`).

use std::sync::Arc;

use chrono::Utc;
use twitch_oauth2::id::DeviceCodeResponse;
use twitch_oauth2::tokens::{DeviceUserTokenBuilder, UserToken};
use twitch_oauth2::types::{AccessToken, ClientId, RefreshToken};
use twitch_oauth2::{Scope, TwitchToken};

use super::errors::AuthError;
use super::storage::TokenStore;
use super::tokens::TwitchTokens;

/// Proactive refresh threshold per ADR 29: refresh if the access token is
/// within this many milliseconds of expiring.
pub const REFRESH_THRESHOLD_MS: i64 = 5 * 60 * 1000;

/// Builder for [`AuthManager`]. Base URLs aren't configurable because
/// `twitch_oauth2` targets the production Twitch endpoints; tests use
/// `mock_api` feature flag (out of scope for this PR).
pub struct AuthManagerBuilder {
    client_id: String,
    scopes: Vec<Scope>,
}

impl AuthManagerBuilder {
    pub fn new(client_id: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            scopes: Vec::new(),
        }
    }

    /// Add a scope to request during the device flow. Scopes aren't
    /// re-requested on refresh — Twitch returns whatever was originally
    /// granted, or a subset.
    #[must_use]
    pub fn scope(mut self, scope: Scope) -> Self {
        self.scopes.push(scope);
        self
    }

    pub fn build<S: TokenStore + 'static>(
        self,
        store: S,
        http_client: reqwest::Client,
    ) -> AuthManager {
        AuthManager {
            client_id: ClientId::new(self.client_id),
            scopes: self.scopes,
            http_client,
            store: Arc::new(store),
        }
    }
}

/// Stateful OAuth + keychain coordinator. Cheap to clone via the internal
/// `Arc<TokenStore>` if the supervisor wants to share one across tasks.
pub struct AuthManager {
    client_id: ClientId,
    scopes: Vec<Scope>,
    http_client: reqwest::Client,
    store: Arc<dyn TokenStore>,
}

impl AuthManager {
    pub fn builder(client_id: impl Into<String>) -> AuthManagerBuilder {
        AuthManagerBuilder::new(client_id)
    }

    /// Returns the shared HTTP client. Callers that need to reuse the
    /// same redirect-disabled configuration (e.g. for other Twitch API
    /// calls) can clone from here rather than building a second one.
    #[must_use]
    pub fn http_client(&self) -> &reqwest::Client {
        &self.http_client
    }

    /// Loads stored tokens, refreshing if within [`REFRESH_THRESHOLD_MS`]
    /// of expiry. The refreshed tokens are persisted (Twitch rotates the
    /// refresh token on every use).
    pub async fn load_or_refresh(&self) -> Result<TwitchTokens, AuthError> {
        let Some(stored) = self.store.load()? else {
            return Err(AuthError::NoTokens);
        };

        if !stored.needs_refresh(Utc::now().timestamp_millis(), REFRESH_THRESHOLD_MS) {
            return Ok(stored);
        }

        let refreshed = self.refresh_tokens(&stored).await?;
        self.store.save(&refreshed)?;
        Ok(refreshed)
    }

    /// Requests a device code from Twitch. The returned response has
    /// `verification_uri` which the caller should open in a browser,
    /// and a `device_code` that [`AuthManager::complete_device_flow`]
    /// needs to pair with.
    ///
    /// The returned builder is consumed on the next call, so start and
    /// complete must happen in order on the same instance.
    pub async fn start_device_flow(&self) -> Result<PendingDeviceFlow, AuthError> {
        let mut builder = DeviceUserTokenBuilder::new(self.client_id.clone(), self.scopes.clone());
        let details = builder
            .start(&self.http_client)
            .await
            .map_err(|e| AuthError::OAuth(e.to_string()))?
            .clone();
        Ok(PendingDeviceFlow { builder, details })
    }

    /// Polls the Twitch token endpoint until the user authorizes the
    /// device code. On success, the resulting tokens (with user_id and
    /// login populated from the DCF response) are persisted.
    pub async fn complete_device_flow(
        &self,
        mut pending: PendingDeviceFlow,
    ) -> Result<TwitchTokens, AuthError> {
        let user_token = pending
            .builder
            .wait_for_code(&self.http_client, tokio::time::sleep)
            .await
            .map_err(classify_device_flow_error)?;
        let tokens = tokens_from_user_token(&user_token);
        self.store.save(&tokens)?;
        Ok(tokens)
    }

    async fn refresh_tokens(&self, stored: &TwitchTokens) -> Result<TwitchTokens, AuthError> {
        // Reconstruct a UserToken from the stored credentials with no
        // secret (public client per ADR 37), then call refresh_token to
        // rotate both access and refresh tokens in-place.
        let mut token = UserToken::from_existing(
            &self.http_client,
            AccessToken::new(stored.access_token.clone()),
            Some(RefreshToken::new(stored.refresh_token.clone())),
            None, // no client secret — public client
        )
        .await
        .map_err(classify_refresh_error)?;

        token
            .refresh_token(&self.http_client)
            .await
            .map_err(classify_refresh_error)?;

        Ok(tokens_from_user_token(&token))
    }
}

/// Opaque handle returned by [`AuthManager::start_device_flow`], carrying
/// the device code response (for UX: show verification URI) and the
/// builder state (for [`AuthManager::complete_device_flow`] to resume
/// polling). A single flow is consumed exactly once.
pub struct PendingDeviceFlow {
    builder: DeviceUserTokenBuilder,
    details: DeviceCodeResponse,
}

impl PendingDeviceFlow {
    #[must_use]
    pub fn details(&self) -> &DeviceCodeResponse {
        &self.details
    }
}

fn tokens_from_user_token(token: &UserToken) -> TwitchTokens {
    let expires_in_ms = i64::try_from(token.expires_in().as_millis()).unwrap_or(i64::MAX);
    let now_ms = Utc::now().timestamp_millis();
    TwitchTokens {
        access_token: token.access_token.secret().to_owned(),
        refresh_token: token
            .refresh_token
            .as_ref()
            .map(|r| r.secret().to_owned())
            .unwrap_or_default(),
        expires_at_ms: now_ms.saturating_add(expires_in_ms),
        scopes: token.scopes().iter().map(|s| s.to_string()).collect(),
        user_id: token.user_id.to_string(),
        login: token.login.to_string(),
    }
}

fn classify_device_flow_error<E: std::fmt::Display>(err: E) -> AuthError {
    let s = err.to_string();
    if s.contains("access_denied") {
        AuthError::UserDenied
    } else if s.contains("expired_token") {
        AuthError::DeviceCodeExpired
    } else {
        AuthError::OAuth(s)
    }
}

fn classify_refresh_error<E: std::fmt::Display>(err: E) -> AuthError {
    let s = err.to_string();
    if s.contains("invalid_grant") || s.contains("Invalid refresh token") {
        AuthError::RefreshTokenInvalid
    } else {
        AuthError::OAuth(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::twitch_auth::storage::MemoryStore;

    fn test_http_client() -> reqwest::Client {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("reqwest client")
    }

    #[test]
    fn classify_device_flow_error_maps_access_denied() {
        match classify_device_flow_error("access_denied") {
            AuthError::UserDenied => {}
            other => panic!("expected UserDenied, got {other:?}"),
        }
    }

    #[test]
    fn classify_device_flow_error_maps_expired_token() {
        match classify_device_flow_error("expired_token while polling") {
            AuthError::DeviceCodeExpired => {}
            other => panic!("expected DeviceCodeExpired, got {other:?}"),
        }
    }

    #[test]
    fn classify_device_flow_error_falls_through_to_oauth() {
        match classify_device_flow_error("some other failure") {
            AuthError::OAuth(s) => assert!(s.contains("some other failure")),
            other => panic!("expected OAuth, got {other:?}"),
        }
    }

    #[test]
    fn classify_refresh_error_maps_invalid_grant() {
        match classify_refresh_error("invalid_grant") {
            AuthError::RefreshTokenInvalid => {}
            other => panic!("expected RefreshTokenInvalid, got {other:?}"),
        }
    }

    #[test]
    fn classify_refresh_error_maps_invalid_refresh_token_phrase() {
        match classify_refresh_error("HTTP 400: Invalid refresh token") {
            AuthError::RefreshTokenInvalid => {}
            other => panic!("expected RefreshTokenInvalid, got {other:?}"),
        }
    }

    #[test]
    fn classify_refresh_error_falls_through_to_oauth() {
        match classify_refresh_error("connection reset by peer") {
            AuthError::OAuth(s) => assert!(s.contains("connection reset")),
            other => panic!("expected OAuth, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn load_or_refresh_returns_no_tokens_when_store_empty() {
        let mgr = AuthManager::builder("test-client-id")
            .build(MemoryStore::default(), test_http_client());

        match mgr.load_or_refresh().await {
            Err(AuthError::NoTokens) => {}
            other => panic!("expected NoTokens, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn load_or_refresh_returns_fresh_tokens_verbatim() {
        // Share the store with the test via a trait wrapper around Arc.
        let store = Arc::new(MemoryStore::default());

        struct Handle(Arc<MemoryStore>);
        impl TokenStore for Handle {
            fn load(&self) -> Result<Option<TwitchTokens>, AuthError> {
                self.0.load()
            }
            fn save(&self, t: &TwitchTokens) -> Result<(), AuthError> {
                self.0.save(t)
            }
            fn delete(&self) -> Result<(), AuthError> {
                self.0.delete()
            }
        }

        let mgr =
            AuthManager::builder("test-client-id").build(Handle(store.clone()), test_http_client());

        let fresh = TwitchTokens {
            access_token: "at-fresh".into(),
            refresh_token: "rt-fresh".into(),
            expires_at_ms: Utc::now().timestamp_millis() + 60 * 60 * 1000,
            scopes: vec!["user:read:chat".into()],
            user_id: "570722168".into(),
            login: "impulseb23".into(),
        };
        store.save(&fresh).unwrap();

        let got = mgr.load_or_refresh().await.unwrap();
        assert_eq!(got, fresh);
    }
}
