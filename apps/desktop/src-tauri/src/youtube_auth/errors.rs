//! Error type shared across the youtube_auth module.
//!
//! Mirrors the variants of `twitch_auth::AuthError` so the frontend's
//! error mapping (and the supervisor's branching) can stay symmetric
//! across providers — only the wire-format details and the underlying
//! flow change between Twitch DCF and YouTube Auth Code + PKCE.

use thiserror::Error;

use crate::oauth_pkce::PkceError;

#[derive(Debug, Error)]
pub enum AuthError {
    /// No tokens have been persisted. Caller's correct response is to
    /// kick off the auth flow.
    #[error("no tokens stored")]
    NoTokens,

    /// Refresh exchange succeeded with the server but the server said
    /// the refresh token is invalid (Google returns `invalid_grant` on
    /// expired/revoked refresh tokens, on consent-revoked, and on the
    /// 6-month-of-inactivity expiry). Per ADR 31 this surfaces a
    /// re-auth UI; do not retry with the same refresh token.
    #[error("refresh token rejected; user must re-authenticate")]
    RefreshTokenInvalid,

    /// User explicitly denied the authorization request (Google sends
    /// `error=access_denied` on the redirect).
    #[error("user denied the authorization")]
    UserDenied,

    /// Loopback listener bind failed — surfaces to the UI so the user
    /// knows it's not their fault when no browser launches.
    #[error("could not start local sign-in listener: {0}")]
    LoopbackBind(String),

    /// CSRF state mismatch between what we sent and what the redirect
    /// returned. Treated as a hard failure — never retry the same
    /// flow; the user should start over.
    #[error("CSRF state mismatch on redirect")]
    StateMismatch,

    /// Keyring / OS credential store error.
    #[error(transparent)]
    Keychain(#[from] keyring::Error),

    /// Any other OAuth / HTTP failure. Carries the upstream message.
    #[error("oauth error: {0}")]
    OAuth(String),

    /// JSON (de)serialization of the persisted token blob failed.
    #[error(transparent)]
    Json(#[from] serde_json::Error),

    /// The /channels?mine=true call returned no items, meaning the
    /// authenticated Google account has no YouTube channel attached.
    /// Real failure mode: people with a Google account but no channel
    /// (rare for streamers, common for general Google users).
    #[error("Google account has no YouTube channel")]
    NoChannel,
}

impl From<PkceError> for AuthError {
    fn from(err: PkceError) -> Self {
        match err {
            PkceError::Authorization(s) if s == "access_denied" => AuthError::UserDenied,
            PkceError::StateMismatch => AuthError::StateMismatch,
            PkceError::Bind(e) => AuthError::LoopbackBind(e.to_string()),
            // `invalid_grant` is Google's universal signal for a dead
            // refresh token on the refresh path. We classify it the
            // same way `twitch_auth` does — it triggers the re-auth UI.
            PkceError::TokenEndpoint(s) if s.contains("invalid_grant") => {
                AuthError::RefreshTokenInvalid
            }
            other => AuthError::OAuth(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_authorization_access_denied_maps_to_user_denied() {
        let err: AuthError = PkceError::Authorization("access_denied".into()).into();
        assert!(matches!(err, AuthError::UserDenied));
    }

    #[test]
    fn pkce_state_mismatch_maps_to_state_mismatch() {
        let err: AuthError = PkceError::StateMismatch.into();
        assert!(matches!(err, AuthError::StateMismatch));
    }

    #[test]
    fn pkce_bind_maps_to_loopback_bind() {
        let io = std::io::Error::new(std::io::ErrorKind::AddrInUse, "in use");
        let err: AuthError = PkceError::Bind(io).into();
        assert!(matches!(err, AuthError::LoopbackBind(_)));
    }

    #[test]
    fn pkce_invalid_grant_maps_to_refresh_token_invalid() {
        let err: AuthError =
            PkceError::TokenEndpoint("Token has been expired or revoked: invalid_grant".into())
                .into();
        assert!(matches!(err, AuthError::RefreshTokenInvalid));
    }

    #[test]
    fn pkce_other_token_endpoint_falls_through_to_oauth() {
        let err: AuthError = PkceError::TokenEndpoint("rate limit exceeded".into()).into();
        match err {
            AuthError::OAuth(msg) => assert!(msg.contains("rate limit")),
            other => panic!("expected OAuth, got {other:?}"),
        }
    }

    #[test]
    fn pkce_authorization_other_error_falls_through_to_oauth() {
        let err: AuthError = PkceError::Authorization("server_error".into()).into();
        match err {
            AuthError::OAuth(msg) => assert!(msg.contains("server_error")),
            other => panic!("expected OAuth, got {other:?}"),
        }
    }
}
