//! Error type shared across the twitch_auth module.
//!
//! Each variant corresponds to a real decision branch a caller needs to
//! make (re-auth UI per ADR 31, retry, surface to the user). Don't add
//! variants unless the caller cares about the distinction.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthError {
    /// No tokens have been persisted for this broadcaster. Supervisor's
    /// correct response is to kick off the device flow.
    #[error("no tokens stored for broadcaster {0}")]
    NoTokens(String),

    /// Refresh exchange succeeded with the server but the server said the
    /// refresh token is invalid. Per ADR 31 this surfaces a re-auth UI;
    /// do not retry with the same refresh token.
    #[error("refresh token rejected; user must re-authenticate")]
    RefreshTokenInvalid,

    /// Device authorization flow ran past its deadline before the user
    /// completed authorization. The caller should start a new flow.
    #[error("device code expired before user authorized")]
    DeviceCodeExpired,

    /// User explicitly denied the authorization request.
    #[error("user denied the device authorization")]
    UserDenied,

    /// Keyring / OS credential store error. Typically means the user
    /// cancelled a prompt or the credential service is unavailable.
    #[error(transparent)]
    Keychain(#[from] keyring::Error),

    /// Any other error from the OAuth stack (HTTP, URL parse, token
    /// response decode, etc.). Callers that need finer-grained handling
    /// can match on `self.source()`.
    #[error("oauth2 stack error: {0}")]
    OAuth(String),

    /// JSON (de)serialization of the persisted token blob failed.
    #[error(transparent)]
    Json(#[from] serde_json::Error),

    /// Configuration error during AuthManager construction (malformed
    /// URL, empty client id, etc.). Bug, not a runtime condition.
    #[error("configuration error: {0}")]
    Config(String),
}
