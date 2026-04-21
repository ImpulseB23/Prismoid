//! Shared OAuth 2.0 Authorization Code + PKCE primitives.
//!
//! Implements the parts of RFC 8252 §7.3 that any platform using
//! loopback IP redirect needs:
//!
//! - PKCE code_verifier + S256 challenge generation (RFC 7636).
//! - CSRF `state` parameter generation.
//! - One-shot HTTP listener bound to `127.0.0.1:0` (OS-picked port)
//!   that captures the redirect query and serves a tiny success page.
//! - Generic POST-form helper for the code-for-token and refresh-token
//!   exchanges.
//!
//! Per-provider concerns (authorization endpoint URL, scope strings,
//! channel/user fetch, refresh-error classification) live in the
//! provider's own module — `youtube_auth` today, `kick_auth` next.
//!
//! Why a separate module: ADR 39 calls out that loopback + PKCE is the
//! shape both YouTube and Kick (write/mod path, ADR 38) need. Mirroring
//! the primitives in two parallel modules would invite drift; mirroring
//! the *flow control* (which scopes, which endpoints) is the part that
//! actually differs by provider.

pub mod errors;
pub mod exchange;
pub mod loopback;
pub mod pkce;

pub use errors::PkceError;
pub use exchange::{exchange_code, refresh_tokens, TokenResponse};
pub use loopback::{LoopbackServer, RedirectParams};
pub use pkce::{Pkce, State};
