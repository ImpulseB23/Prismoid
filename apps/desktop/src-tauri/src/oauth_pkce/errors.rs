//! PKCE error type. Variants exist where a caller actually branches.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PkceError {
    /// TCP listener could not bind `127.0.0.1:0`. Practically this means
    /// the loopback interface is firewalled in a way the OS itself
    /// rejects — extremely rare on a developer/streamer machine, but
    /// surfaced so the UI can tell the user instead of hanging.
    #[error("failed to bind loopback listener: {0}")]
    Bind(#[source] std::io::Error),

    /// Listener bound but accepting / reading the inbound HTTP request
    /// failed.
    #[error("loopback I/O error: {0}")]
    Io(#[source] std::io::Error),

    /// Inbound request did not look like the expected `GET /?...` from
    /// the OAuth provider's redirect. Typically a probe (browser
    /// pre-fetch, port scanner) — caller should keep waiting; the
    /// listener only resolves on the *first valid* request.
    #[error("malformed redirect request: {0}")]
    BadRequest(&'static str),

    /// Redirect carried an `error=` query parameter from the provider.
    /// The caller maps this to `UserDenied` / generic `OAuth` based on
    /// the value (e.g. `access_denied`).
    #[error("authorization endpoint returned error: {0}")]
    Authorization(String),

    /// `state` parameter on the redirect did not match what we sent.
    /// CSRF defense per RFC 6749 §10.12 — fail closed.
    #[error("state mismatch on redirect")]
    StateMismatch,

    /// Provider's token endpoint rejected the exchange. Body carries
    /// `error_description` when available so the user / log sees why.
    #[error("token endpoint error: {0}")]
    TokenEndpoint(String),

    /// HTTP transport failure during code-for-token or refresh.
    #[error("token endpoint HTTP error: {0}")]
    Http(String),

    /// Token endpoint returned 200 but the JSON didn't decode into the
    /// expected shape. Usually a sign the endpoint URL is wrong or the
    /// provider changed their response format.
    #[error("token response decode error: {0}")]
    Decode(String),
}
