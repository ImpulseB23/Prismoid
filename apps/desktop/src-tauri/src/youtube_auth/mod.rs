//! YouTube OAuth + keychain integration.
//!
//! Implements ADR 39 (YouTube OAuth 2.0 via Authorization Code + PKCE
//! with loopback IP redirect, tokens via `keyring-rs`), ADR 29
//! (proactive refresh 5 min before expiry), and ADR 31 (re-auth path
//! on refresh failure). The flow:
//!
//! 1. At startup the supervisor calls [`AuthManager::load_or_refresh`].
//!    If fresh, the access token is handed to the sidecar via
//!    `youtube_connect`. If stale, the manager refreshes transparently
//!    and persists the rotated refresh token.
//! 2. On [`AuthError::NoTokens`] (first run) or
//!    [`AuthError::RefreshTokenInvalid`] (Google's refresh tokens
//!    silently expire after 6 months of inactivity, plus other invalid
//!    states), the frontend kicks [`AuthManager::start_login`] →
//!    [`AuthManager::complete_login`].
//!
//! The module is pure-logic and async-only; wiring into the supervisor
//! lives alongside the Twitch supervisor in `lib.rs::setup`.

pub mod auth_state;
pub mod commands;
pub mod errors;
pub mod manager;
pub mod storage;
pub mod tokens;

pub use auth_state::{AuthCommandError, AuthState, AuthStatus, AuthStatusState, PkceFlowView};
pub use commands::{
    youtube_auth_status, youtube_cancel_login, youtube_complete_login, youtube_logout,
    youtube_start_login,
};
pub use errors::AuthError;
pub use manager::{AuthManager, AuthManagerBuilder, PendingLogin, REFRESH_THRESHOLD_MS};
pub use storage::{KeychainStore, MemoryStore, TokenStore, KEYCHAIN_ACCOUNT, KEYCHAIN_SERVICE};
pub use tokens::YouTubeTokens;

/// OAuth `client_id` for the registered Prismoid Google application.
///
/// Sourced from the `GOOGLE_CLIENT_ID` env var at compile time so the
/// real Desktop client credential never lands in the public repo.
/// Falls back to a placeholder when the env var is unset *or empty*
/// (GitHub Actions expands a missing `${{ secrets.X }}` to `""`, and
/// `option_env!` returns `Some("")` in that case), in which case the
/// surrounding code paths (start_login → complete_login → exchange)
/// all return `AuthError::OAuth("invalid_client")`.
///
/// Per RFC 8252 §8.4 and Google's own docs, this `client_id` is a
/// public identifier — it appears in browser URLs during the
/// authorization flow and is bundled in source the same way the
/// Twitch DCF flow handles `TWITCH_CLIENT_ID` (see ADR 37).
pub const GOOGLE_CLIENT_ID: &str = or_placeholder(
    option_env!("GOOGLE_CLIENT_ID"),
    "REPLACE_ME.apps.googleusercontent.com",
);

/// OAuth `client_secret` for the registered Prismoid Google application.
///
/// Sourced from the `GOOGLE_CLIENT_SECRET` env var at compile time.
/// Empty values are treated as unset (see [`GOOGLE_CLIENT_ID`] for the
/// rationale). Google issues a `client_secret` for "Desktop app"
/// credentials and requires it on the token-exchange POST, but their
/// own [installed-app docs](https://developers.google.com/identity/protocols/oauth2/native-app)
/// note: *"In this context, the client secret is obviously not treated
/// as a secret."* PKCE S256 is what cryptographically protects the
/// flow on a public client; this string is included on the wire only
/// because Google's endpoint won't accept the request without it.
pub const GOOGLE_CLIENT_SECRET: &str =
    or_placeholder(option_env!("GOOGLE_CLIENT_SECRET"), "REPLACE_ME");

/// Returns `env` when it's a non-empty string, otherwise `default`.
/// Used so a build env var explicitly set to `""` (the GitHub Actions
/// expansion of a missing secret) is treated as unset rather than
/// silently embedding empty credentials in the binary.
const fn or_placeholder(env: Option<&'static str>, default: &'static str) -> &'static str {
    match env {
        Some(v) if !v.is_empty() => v,
        _ => default,
    }
}

/// Google OAuth 2.0 authorization endpoint. Hard-coded to the v2
/// endpoint per Google's [installed-app guide](https://developers.google.com/identity/protocols/oauth2/native-app#step-2-send-a-request-to-googles-oauth-20-server).
pub const GOOGLE_AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";

/// Google OAuth 2.0 token endpoint. Hard-coded to the v4 endpoint per
/// the same guide.
pub const GOOGLE_TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";

/// YouTube Data API v3 channels endpoint, used post-token-exchange to
/// fetch the authenticated user's channel id + title.
pub const YOUTUBE_CHANNELS_ENDPOINT: &str = "https://www.googleapis.com/youtube/v3/channels";

/// Read-only YouTube Data API scope. Required to read live chat.
pub const SCOPE_YOUTUBE_READONLY: &str = "https://www.googleapis.com/auth/youtube.readonly";

/// Full YouTube scope (read + write + moderation). Required to send
/// messages and ban/timeout/delete on YouTube live chat.
pub const SCOPE_YOUTUBE: &str = "https://www.googleapis.com/auth/youtube";

#[cfg(test)]
mod tests {
    use super::or_placeholder;

    #[test]
    fn or_placeholder_uses_env_when_non_empty() {
        assert_eq!(or_placeholder(Some("value"), "fallback"), "value");
    }

    #[test]
    fn or_placeholder_falls_back_when_unset() {
        assert_eq!(or_placeholder(None, "fallback"), "fallback");
    }

    #[test]
    fn or_placeholder_falls_back_when_empty() {
        // GitHub Actions expands a missing `${{ secrets.X }}` to `""`
        // and option_env! surfaces that as Some(""). The helper must
        // treat it as unset so release builds don't embed empty creds.
        assert_eq!(or_placeholder(Some(""), "fallback"), "fallback");
    }
}
