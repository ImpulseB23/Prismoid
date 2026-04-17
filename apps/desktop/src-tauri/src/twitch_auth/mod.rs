//! Twitch OAuth + keychain integration.
//!
//! Implements ADR 37 (Twitch Device Code Grant public client, tokens via
//! `keyring-rs`), ADR 29 (proactive refresh 5 min before expiry), and
//! ADR 31 (re-auth path on refresh failure). The flow:
//!
//! 1. At startup the supervisor calls [`AuthManager::load_or_refresh`]
//!    for the broadcaster. If fresh, the access token is handed to the
//!    sidecar via `twitch_connect`. If stale, the manager refreshes
//!    transparently and persists the rotated refresh token.
//! 2. On [`AuthError::NoTokens`] (first run / keychain cleared) or
//!    [`AuthError::RefreshTokenInvalid`] (30-day inactive expiry), the
//!    frontend kicks [`AuthManager::start_device_flow`] →
//!    [`AuthManager::complete_device_flow`].
//!
//! The module is pure-logic and async-only; wiring into the supervisor
//! lives in PRI-21.

pub mod auth_state;
pub mod commands;
pub mod errors;
pub mod manager;
pub mod storage;
pub mod tokens;

pub use auth_state::{AuthCommandError, AuthState, AuthStatus, AuthStatusState, DeviceCodeView};
pub use commands::{
    twitch_auth_status, twitch_cancel_login, twitch_complete_login, twitch_logout,
    twitch_start_login,
};
pub use errors::AuthError;
pub use manager::{AuthManager, AuthManagerBuilder, PendingDeviceFlow, REFRESH_THRESHOLD_MS};
pub use storage::{KeychainStore, MemoryStore, TokenStore, KEYCHAIN_SERVICE};
pub use tokens::TwitchTokens;

/// OAuth `client_id` for the registered Prismoid Twitch application.
///
/// **This is intentionally a plain string literal, not a secret.** Per
/// RFC 8252 §8.4 and RFC 6749 §2.2, OAuth public-client `client_id`s
/// are public identifiers — they appear in browser URLs and network
/// traces during the authorization flow. Treating them as secret is a
/// category error.
///
/// Bundling the production `client_id` in source is the standard pattern
/// for distributed desktop apps doing OAuth public-client flows
/// (`github/cli`, Discord/Slack/Spotify desktop, etc.). Forks running
/// against their own registered Twitch application override this const
/// at build time.
pub const TWITCH_CLIENT_ID: &str = "bpjpbhc5p8xicpiuvxskjuq4m9gcio";
