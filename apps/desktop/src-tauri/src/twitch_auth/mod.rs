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

pub mod errors;
pub mod manager;
pub mod storage;
pub mod tokens;

pub use errors::AuthError;
pub use manager::{AuthManager, AuthManagerBuilder, PendingDeviceFlow, REFRESH_THRESHOLD_MS};
pub use storage::{KeychainStore, MemoryStore, TokenStore, KEYCHAIN_SERVICE};
pub use tokens::TwitchTokens;
