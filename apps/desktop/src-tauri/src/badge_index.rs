//! Channel-scoped badge index.
//!
//! Mirror of [`crate::emote_index::EmoteIndex`] for chat badges. The
//! sidecar delivers the full badge catalogue in an [`EmoteBundle`] once
//! per channel join; [`BadgeIndex::resolve_into`] then enriches each
//! incoming [`message::Badge`] with URLs + title so the frontend can
//! render the badge image without a second round trip.
//!
//! Design notes:
//! - Lock-free reads via [`ArcSwap`], identical pattern to `EmoteIndex`.
//! - Nested `(set_id -> id -> Arc<Badge>)` map keeps lookups allocation-
//!   free: `&str` keys borrow through `Box<str>` without materializing an
//!   owned key per message.
//! - Channel badges override global badges on key collision
//!   (Chatterino convention — custom subscriber art on a channel wins
//!   over Twitch's default subscriber art).

use std::sync::Arc;

use arc_swap::ArcSwap;
use rustc_hash::FxHashMap;

use crate::emote_index::{Badge, EmoteBundle};
use crate::message;

struct Snapshot {
    by_set: FxHashMap<Box<str>, FxHashMap<Box<str>, Arc<Badge>>>,
}

impl Snapshot {
    fn empty() -> Self {
        Self {
            by_set: FxHashMap::default(),
        }
    }
}

pub struct BadgeIndex {
    inner: ArcSwap<Snapshot>,
}

impl BadgeIndex {
    pub fn new() -> Self {
        Self {
            inner: ArcSwap::from_pointee(Snapshot::empty()),
        }
    }

    /// Replace the current snapshot with the badges in `bundle`. Global
    /// badges are ingested first and channel badges second so that any
    /// channel override replaces the global entry for the same
    /// `(set, version)` key.
    pub fn load_bundle(&self, bundle: &EmoteBundle) {
        let mut by_set: FxHashMap<Box<str>, FxHashMap<Box<str>, Arc<Badge>>> = FxHashMap::default();
        ingest(&mut by_set, &bundle.twitch_global_badges.badges);
        ingest(&mut by_set, &bundle.twitch_channel_badges.badges);
        self.inner.store(Arc::new(Snapshot { by_set }));
    }

    /// Mutates each badge in `badges` in place, filling in the resolved
    /// URL + title fields when the badge's `(set_id, id)` pair is known.
    /// Unknown badges are left untouched — the frontend store falls back
    /// to not rendering them.
    pub fn resolve_into(&self, badges: &mut [message::Badge]) {
        let snap = self.inner.load();
        if snap.by_set.is_empty() {
            return;
        }
        for b in badges.iter_mut() {
            let Some(versions) = snap.by_set.get(b.set_id.as_str()) else {
                continue;
            };
            let Some(resolved) = versions.get(b.id.as_str()) else {
                continue;
            };
            b.title = resolved.title.to_string();
            b.url_1x = resolved.url_1x.to_string();
            b.url_2x = resolved.url_2x.to_string();
            b.url_4x = resolved.url_4x.to_string();
        }
    }

    /// Total badges across all sets. Used for logging.
    pub fn len(&self) -> usize {
        self.inner.load().by_set.values().map(|v| v.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for BadgeIndex {
    fn default() -> Self {
        Self::new()
    }
}

fn ingest(target: &mut FxHashMap<Box<str>, FxHashMap<Box<str>, Arc<Badge>>>, badges: &[Badge]) {
    for b in badges {
        let versions = target.entry(b.set.clone()).or_default();
        versions.insert(b.version.clone(), Arc::new(b.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emote_index::BadgeSet;

    fn badge(set: &str, version: &str, suffix: &str) -> Badge {
        Badge {
            set: set.into(),
            version: version.into(),
            title: format!("{set}/{version}{suffix}").into(),
            url_1x: format!("https://cdn/{set}/{version}/1x{suffix}.png").into(),
            url_2x: format!("https://cdn/{set}/{version}/2x{suffix}.png").into(),
            url_4x: format!("https://cdn/{set}/{version}/4x{suffix}.png").into(),
        }
    }

    fn bundle(global: Vec<Badge>, channel: Vec<Badge>) -> EmoteBundle {
        EmoteBundle {
            twitch_global_badges: BadgeSet { badges: global },
            twitch_channel_badges: BadgeSet { badges: channel },
            ..Default::default()
        }
    }

    fn msg_badge(set_id: &str, id: &str) -> message::Badge {
        message::Badge {
            set_id: set_id.into(),
            id: id.into(),
            ..Default::default()
        }
    }

    #[test]
    fn resolves_a_known_global_badge() {
        let idx = BadgeIndex::new();
        idx.load_bundle(&bundle(vec![badge("moderator", "1", "")], vec![]));
        let mut badges = vec![msg_badge("moderator", "1")];
        idx.resolve_into(&mut badges);
        assert_eq!(badges[0].url_1x, "https://cdn/moderator/1/1x.png");
        assert_eq!(badges[0].title, "moderator/1");
    }

    #[test]
    fn channel_overrides_global() {
        let idx = BadgeIndex::new();
        idx.load_bundle(&bundle(
            vec![badge("subscriber", "0", "-global")],
            vec![badge("subscriber", "0", "-channel")],
        ));
        let mut badges = vec![msg_badge("subscriber", "0")];
        idx.resolve_into(&mut badges);
        assert_eq!(badges[0].title, "subscriber/0-channel");
    }

    #[test]
    fn unknown_badge_is_left_untouched() {
        let idx = BadgeIndex::new();
        idx.load_bundle(&bundle(vec![badge("moderator", "1", "")], vec![]));
        let mut badges = vec![msg_badge("vip", "1")];
        idx.resolve_into(&mut badges);
        assert!(badges[0].url_1x.is_empty());
        assert!(badges[0].title.is_empty());
    }

    #[test]
    fn empty_index_is_a_noop() {
        let idx = BadgeIndex::new();
        let mut badges = vec![msg_badge("moderator", "1")];
        idx.resolve_into(&mut badges);
        assert!(badges[0].url_1x.is_empty());
    }

    #[test]
    fn reload_replaces_previous_snapshot() {
        let idx = BadgeIndex::new();
        idx.load_bundle(&bundle(vec![badge("vip", "1", "")], vec![]));
        idx.load_bundle(&bundle(vec![badge("moderator", "1", "")], vec![]));
        let mut badges = vec![msg_badge("vip", "1"), msg_badge("moderator", "1")];
        idx.resolve_into(&mut badges);
        assert!(badges[0].url_1x.is_empty());
        assert!(!badges[1].url_1x.is_empty());
    }
}
