//! Persistable token DTO.
//!
//! [`oauth2`] provides its own `BasicTokenResponse` at runtime, but that
//! type isn't designed for serde-to-disk (secrets are wrapped opaquely and
//! the shape isn't a stable wire format). `TwitchTokens` is what we
//! actually store in the keychain: flat strings + an absolute expiry.

use serde::{Deserialize, Serialize};

/// A persisted Twitch OAuth credential set. One of these per broadcaster
/// lives as a JSON blob in the keychain under service `prismoid.twitch`
/// with account `<broadcaster_id>` (see ADR 37).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TwitchTokens {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix milliseconds at which `access_token` expires. Absolute time
    /// is captured at save, not expires_in, so a sidecar that's been
    /// asleep for hours still evaluates freshness correctly on wake.
    pub expires_at_ms: i64,
    /// Scopes granted. Carried so a scope-expansion feature can prompt
    /// re-auth when needed without speculative re-auth on every launch.
    pub scopes: Vec<String>,
}

impl TwitchTokens {
    /// Returns true if the access token is either already expired or
    /// within `threshold_ms` of expiring — either way the caller should
    /// refresh before using it. ADR 29 pins the threshold to 5 min.
    #[must_use]
    pub fn needs_refresh(&self, now_ms: i64, threshold_ms: i64) -> bool {
        now_ms + threshold_ms >= self.expires_at_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(expires_at_ms: i64) -> TwitchTokens {
        TwitchTokens {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            expires_at_ms,
            scopes: vec!["user:read:chat".into()],
        }
    }

    #[test]
    fn needs_refresh_fresh_token_returns_false() {
        let t = sample(1_000_000);
        assert!(!t.needs_refresh(0, 300_000));
    }

    #[test]
    fn needs_refresh_exactly_at_threshold_returns_true() {
        // now + threshold == expires_at is the boundary: the token has
        // exactly `threshold` ms left. Refresh now, not later.
        let t = sample(1_000_000);
        assert!(t.needs_refresh(700_000, 300_000));
    }

    #[test]
    fn needs_refresh_one_ms_before_threshold_returns_false() {
        let t = sample(1_000_000);
        assert!(!t.needs_refresh(699_999, 300_000));
    }

    #[test]
    fn needs_refresh_already_expired_returns_true() {
        let t = sample(500_000);
        assert!(t.needs_refresh(1_000_000, 0));
    }

    #[test]
    fn roundtrip_json_preserves_all_fields() {
        let t = sample(1_234_567);
        let blob = serde_json::to_string(&t).unwrap();
        let back: TwitchTokens = serde_json::from_str(&blob).unwrap();
        assert_eq!(t, back);
    }
}
