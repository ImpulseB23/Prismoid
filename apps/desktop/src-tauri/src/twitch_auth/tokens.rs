//! Persistable token DTO.
//!
//! [`oauth2`] provides its own `BasicTokenResponse` at runtime, but that
//! type isn't designed for serde-to-disk (secrets are wrapped opaquely and
//! the shape isn't a stable wire format). `TwitchTokens` is what we
//! actually store in the keychain: flat strings + an absolute expiry.

use serde::{Deserialize, Serialize};

/// A persisted Twitch OAuth credential set. Single-account per ADR 30:
/// one blob per app under service `prismoid.twitch`, account
/// `KEYCHAIN_ACCOUNT` (see `storage.rs`). `user_id` and `login` are the
/// authenticated Twitch user's identifiers (from the DCF response) —
/// carried with the token so the supervisor doesn't need to prompt the
/// user for their own broadcaster ID.
///
/// `Debug` is hand-rolled to redact token secrets. Any accidental
/// `tracing::debug!("{tokens:?}")` or similar at a call site must not
/// leak the access/refresh tokens to log files. Same pattern oauth2's
/// `AccessToken` type uses.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
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
    /// Twitch user_id of the authenticated account, from the DCF
    /// response. Used as `broadcaster_user_id` in EventSub subscriptions.
    /// Stable across token refreshes.
    pub user_id: String,
    /// Twitch login (handle) of the authenticated account. Used for UI
    /// display ("Logged in as @<login>") and diagnostic logs. Stable
    /// across token refreshes.
    pub login: String,
}

impl std::fmt::Debug for TwitchTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TwitchTokens")
            .field("access_token", &"[redacted]")
            .field("refresh_token", &"[redacted]")
            .field("expires_at_ms", &self.expires_at_ms)
            .field("scopes", &self.scopes)
            .field("user_id", &self.user_id)
            .field("login", &self.login)
            .finish()
    }
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
            user_id: "123".into(),
            login: "someone".into(),
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

    #[test]
    fn debug_impl_redacts_secrets() {
        let t = TwitchTokens {
            access_token: "super-secret-access-xyz".into(),
            refresh_token: "super-secret-refresh-xyz".into(),
            expires_at_ms: 1_234_567,
            scopes: vec!["user:read:chat".into()],
            user_id: "570722168".into(),
            login: "impulseb23".into(),
        };
        let debug_str = format!("{t:?}");
        assert!(
            !debug_str.contains("super-secret-access-xyz"),
            "access_token leaked through Debug: {debug_str}"
        );
        assert!(
            !debug_str.contains("super-secret-refresh-xyz"),
            "refresh_token leaked through Debug: {debug_str}"
        );
        assert!(debug_str.contains("[redacted]"));
        // Non-secret fields still observable for debugging.
        assert!(debug_str.contains("1234567"));
        assert!(debug_str.contains("user:read:chat"));
        assert!(debug_str.contains("570722168"));
        assert!(debug_str.contains("impulseb23"));
    }
}
