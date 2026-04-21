//! Persistable token DTO for YouTube.
//!
//! Mirrors `twitch_auth::TwitchTokens` shape with the YouTube-specific
//! identity fields (`channel_id`, `channel_title`) substituted for
//! Twitch's `user_id`/`login`. Same redacted Debug pattern.

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct YouTubeTokens {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix milliseconds at which `access_token` expires. Absolute time
    /// is captured at save, not expires_in, so a sleeping process still
    /// evaluates freshness correctly on wake.
    pub expires_at_ms: i64,
    /// Scopes granted, parsed from the token endpoint's space-delimited
    /// `scope` response field.
    pub scopes: Vec<String>,
    /// YouTube channel id (the `UC...` ID) of the authenticated user,
    /// fetched once from `youtube.channels?mine=true` after the token
    /// exchange. Used as `liveChatId` resolver input and as the
    /// "Logged in as" display anchor.
    pub channel_id: String,
    /// Channel display name from `snippet.title`. UI display only.
    pub channel_title: String,
}

impl std::fmt::Debug for YouTubeTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("YouTubeTokens")
            .field("access_token", &"[redacted]")
            .field("refresh_token", &"[redacted]")
            .field("expires_at_ms", &self.expires_at_ms)
            .field("scopes", &self.scopes)
            .field("channel_id", &self.channel_id)
            .field("channel_title", &self.channel_title)
            .finish()
    }
}

impl YouTubeTokens {
    /// Returns true if the access token is either already expired or
    /// within `threshold_ms` of expiring. ADR 29 pins the threshold to
    /// 5 min.
    #[must_use]
    pub fn needs_refresh(&self, now_ms: i64, threshold_ms: i64) -> bool {
        now_ms + threshold_ms >= self.expires_at_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(expires_at_ms: i64) -> YouTubeTokens {
        YouTubeTokens {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            expires_at_ms,
            scopes: vec!["https://www.googleapis.com/auth/youtube.readonly".into()],
            channel_id: "UC123".into(),
            channel_title: "Test Channel".into(),
        }
    }

    #[test]
    fn needs_refresh_fresh_token_returns_false() {
        let t = sample(1_000_000);
        assert!(!t.needs_refresh(0, 300_000));
    }

    #[test]
    fn needs_refresh_at_threshold_returns_true() {
        let t = sample(1_000_000);
        assert!(t.needs_refresh(700_000, 300_000));
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
        let back: YouTubeTokens = serde_json::from_str(&blob).unwrap();
        assert_eq!(t, back);
    }

    #[test]
    fn debug_impl_redacts_secrets() {
        let t = YouTubeTokens {
            access_token: "super-secret-access-xyz".into(),
            refresh_token: "super-secret-refresh-xyz".into(),
            expires_at_ms: 1_234_567,
            scopes: vec![],
            channel_id: "UCabc".into(),
            channel_title: "Title".into(),
        };
        let dbg = format!("{t:?}");
        assert!(!dbg.contains("super-secret"));
        assert!(dbg.contains("[redacted]"));
        assert!(dbg.contains("UCabc"));
    }
}
