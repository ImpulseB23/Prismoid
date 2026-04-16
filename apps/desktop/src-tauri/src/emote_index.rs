//! Channel-scoped emote index (ADR 13).
//!
//! Frontend-agnostic, lock-free-on-read catalog of emote metadata plus an
//! aho-corasick automaton over emote codes. The host swaps a whole new
//! snapshot whenever emotes change (channel join, 7TV set reload); readers
//! on the message hot path take an [`arc_swap::Guard`] and scan without
//! blocking.
//!
//! Not responsible for fetching catalogs (that's `sidecar/internal/emotes`)
//! or for caching emote image bytes (that's the SQLite cache, follow-up).

use std::sync::Arc;

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
use arc_swap::ArcSwap;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use tracing::warn;

/// Emote provider. Mirrors `sidecar/internal/emotes.Provider`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Twitch,
    #[serde(rename = "7tv")]
    SevenTv,
    Bttv,
    Ffz,
}

/// Normalized metadata for a single emote. Fields match the Go side
/// (`sidecar/internal/emotes.Emote`) so the sidecar can write these directly
/// over the control plane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmoteMeta {
    pub id: Box<str>,
    /// Emote code (what users type). Stored as `Arc<str>` so the index can
    /// share a single allocation between the lookup map key and the meta.
    pub code: Arc<str>,
    pub provider: Provider,
    #[serde(rename = "url_1x")]
    pub url_1x: Box<str>,
    #[serde(rename = "url_2x", default)]
    pub url_2x: Box<str>,
    #[serde(rename = "url_4x", default)]
    pub url_4x: Box<str>,
    #[serde(default)]
    pub width: u16,
    #[serde(default)]
    pub height: u16,
    #[serde(default)]
    pub animated: bool,
    #[serde(default)]
    pub zero_width: bool,
}

/// One provider+scope slice of an [`EmoteBundle`]. Mirrors
/// `sidecar/internal/emotes.EmoteSet`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EmoteSet {
    #[serde(default)]
    pub emotes: Vec<EmoteMeta>,
}

/// The full per-channel emote catalog as delivered by the sidecar in a
/// single `emote_bundle` control message. Only the emote fields are
/// consumed by [`EmoteIndex::load_bundle`]; badges are carried through for
/// the frontend but do not participate in text scanning. Unknown fields
/// (badges, errors) are ignored here and deserialized separately where
/// needed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EmoteBundle {
    #[serde(default)]
    pub twitch_global_emotes: EmoteSet,
    #[serde(default)]
    pub twitch_channel_emotes: EmoteSet,
    #[serde(default)]
    pub seventv_global: EmoteSet,
    #[serde(default)]
    pub seventv_channel: EmoteSet,
    #[serde(default)]
    pub bttv_global: EmoteSet,
    #[serde(default)]
    pub bttv_channel: EmoteSet,
    #[serde(default)]
    pub ffz_global: EmoteSet,
    #[serde(default)]
    pub ffz_channel: EmoteSet,
}

impl EmoteBundle {
    /// Iterates every emote in the order [`EmoteIndex::load_bundle`] uses
    /// to resolve duplicate codes: global first, then channel, with
    /// third-party providers overriding Twitch within each scope. The
    /// Chatterino convention, and the expected one for users coming from
    /// other Twitch chat clients.
    fn iter_in_precedence_order(self) -> impl Iterator<Item = EmoteMeta> {
        self.twitch_global_emotes
            .emotes
            .into_iter()
            .chain(self.bttv_global.emotes)
            .chain(self.ffz_global.emotes)
            .chain(self.seventv_global.emotes)
            .chain(self.twitch_channel_emotes.emotes)
            .chain(self.bttv_channel.emotes)
            .chain(self.ffz_channel.emotes)
            .chain(self.seventv_channel.emotes)
    }

    /// Total emote count across all eight sets. Exposed for logging + the
    /// frontend status UI, not used on the hot path.
    pub fn total_emotes(&self) -> usize {
        self.twitch_global_emotes.emotes.len()
            + self.twitch_channel_emotes.emotes.len()
            + self.seventv_global.emotes.len()
            + self.seventv_channel.emotes.len()
            + self.bttv_global.emotes.len()
            + self.bttv_channel.emotes.len()
            + self.ffz_global.emotes.len()
            + self.ffz_channel.emotes.len()
    }
}

/// Byte range of a matched emote code inside a message's `message_text`,
/// plus the resolved emote metadata. `start..end` is a UTF-8 byte slice of
/// the original text.
#[derive(Debug, Clone, Serialize)]
pub struct EmoteSpan {
    pub start: u32,
    pub end: u32,
    pub emote: Arc<EmoteMeta>,
}

/// Immutable snapshot of the index. Built once by [`EmoteIndex::load`] and
/// pointed at by the atomic swap; readers keep the snapshot alive for the
/// duration of their scan regardless of subsequent rebuilds.
struct Snapshot {
    by_code: FxHashMap<Arc<str>, Arc<EmoteMeta>>,
    /// aho-corasick pattern i resolves to `patterns[i]`. Kept parallel so
    /// the automaton output carries zero per-match metadata.
    patterns: Vec<Arc<EmoteMeta>>,
    /// `None` when there are zero emotes, or when the builder rejected the
    /// input (all codes filtered out as invalid).
    ac: Option<AhoCorasick>,
}

impl Snapshot {
    fn empty() -> Self {
        Self {
            by_code: FxHashMap::default(),
            patterns: Vec::new(),
            ac: None,
        }
    }
}

/// Lock-free emote index. Wrap in [`Arc<EmoteIndex>`] when callers need
/// cheap shared ownership; typically one instance per active channel.
pub struct EmoteIndex {
    inner: ArcSwap<Snapshot>,
}

impl EmoteIndex {
    pub fn new() -> Self {
        Self {
            inner: ArcSwap::from_pointee(Snapshot::empty()),
        }
    }

    /// Replace the current snapshot with one built from `emotes`. Duplicate
    /// codes are resolved by "later wins" — callers that care about
    /// precedence (channel > global, or 7TV > BTTV > FFZ) pass entries in
    /// ascending priority order so the highest-priority source overwrites.
    ///
    /// Emotes with empty codes are dropped (aho-corasick rejects them and
    /// they would never match anyway).
    pub fn load<I: IntoIterator<Item = EmoteMeta>>(&self, emotes: I) {
        let mut by_code: FxHashMap<Arc<str>, Arc<EmoteMeta>> = FxHashMap::default();
        for e in emotes {
            if e.code.is_empty() {
                continue;
            }
            let code = Arc::clone(&e.code);
            by_code.insert(code, Arc::new(e));
        }

        let mut patterns: Vec<Arc<EmoteMeta>> = Vec::with_capacity(by_code.len());
        let mut needles: Vec<&str> = Vec::with_capacity(by_code.len());
        for e in by_code.values() {
            patterns.push(Arc::clone(e));
            needles.push(&e.code);
        }

        // LeftmostLongest matches Chatterino's resolution: when multiple
        // codes share a prefix (`Kappa` / `KappaHD`), the longer code wins
        // at the same starting offset.
        let ac = if needles.is_empty() {
            None
        } else {
            match AhoCorasickBuilder::new()
                .match_kind(MatchKind::LeftmostLongest)
                .build(&needles)
            {
                Ok(ac) => Some(ac),
                Err(err) => {
                    warn!(error = %err, count = needles.len(), "emote_index: aho-corasick build failed; scans will no-op until next reload");
                    None
                }
            }
        };

        self.inner.store(Arc::new(Snapshot {
            by_code,
            patterns,
            ac,
        }));
    }

    /// Replace the current snapshot with one built from an [`EmoteBundle`]
    /// delivered by the sidecar. Equivalent to calling [`load`](Self::load)
    /// with the bundle's emotes in the documented precedence order so
    /// channel-scoped emotes and third-party providers win over Twitch
    /// globals.
    pub fn load_bundle(&self, bundle: EmoteBundle) {
        self.load(bundle.iter_in_precedence_order());
    }

    /// Look up an emote by its exact code. Case-sensitive — Twitch and
    /// third-party providers treat codes as case-sensitive identifiers.
    pub fn lookup(&self, code: &str) -> Option<Arc<EmoteMeta>> {
        self.inner.load().by_code.get(code).cloned()
    }

    /// Current number of indexed emotes. Snapshot-consistent.
    pub fn len(&self) -> usize {
        self.inner.load().by_code.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Scan `text` for emote matches and append them to `out`. The caller
    /// owns the buffer so the message hot path can reuse a scratch `Vec`
    /// across parses and avoid per-message allocation.
    ///
    /// Matches are emitted left-to-right and only when the match is bounded
    /// by ASCII whitespace or the ends of `text` — this mirrors how Twitch
    /// chat tokenizes, and prevents `Kappa` inside `KappaPride` (if only
    /// `Kappa` is indexed) from matching as a substring.
    ///
    /// Texts longer than `u32::MAX` bytes are not scanned; chat messages
    /// are capped far below that in practice, so this only guards against
    /// corrupt input.
    pub fn scan_into(&self, text: &str, out: &mut Vec<EmoteSpan>) {
        if text.len() > u32::MAX as usize {
            return;
        }
        let snap = self.inner.load();
        let Some(ac) = snap.ac.as_ref() else {
            return;
        };
        let bytes = text.as_bytes();
        for m in ac.find_iter(text) {
            let start = m.start();
            let end = m.end();
            if !is_token_boundary(bytes, start, end) {
                continue;
            }
            let emote = Arc::clone(&snap.patterns[m.pattern().as_usize()]);
            out.push(EmoteSpan {
                start: start as u32,
                end: end as u32,
                emote,
            });
        }
    }
}

impl Default for EmoteIndex {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns true when `start..end` is surrounded by ASCII whitespace (space,
/// tab, newline, carriage return, form feed) or sits at a string boundary.
/// Emote codes never contain whitespace, so whitespace is the only valid
/// delimiter across Twitch, 7TV, BTTV, and FFZ.
fn is_token_boundary(bytes: &[u8], start: usize, end: usize) -> bool {
    let left_ok = start == 0 || bytes[start - 1].is_ascii_whitespace();
    let right_ok = end == bytes.len() || bytes[end].is_ascii_whitespace();
    left_ok && right_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(code: &str, provider: Provider) -> EmoteMeta {
        EmoteMeta {
            id: code.into(),
            code: code.into(),
            provider,
            url_1x: format!("https://cdn/{code}/1x").into(),
            url_2x: "".into(),
            url_4x: "".into(),
            width: 28,
            height: 28,
            animated: false,
            zero_width: false,
        }
    }

    #[test]
    fn empty_index_scans_cleanly() {
        let idx = EmoteIndex::new();
        let mut out = Vec::new();
        idx.scan_into("Kappa PogChamp", &mut out);
        assert!(out.is_empty());
        assert_eq!(idx.len(), 0);
        assert!(idx.is_empty());
    }

    #[test]
    fn exact_and_tokenized_match() {
        let idx = EmoteIndex::new();
        idx.load([
            meta("Kappa", Provider::Twitch),
            meta("PogChamp", Provider::Twitch),
        ]);

        let mut out = Vec::new();
        idx.scan_into("hello Kappa and PogChamp world", &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(
            &"hello Kappa and PogChamp world"[out[0].start as usize..out[0].end as usize],
            "Kappa"
        );
        assert_eq!(
            &"hello Kappa and PogChamp world"[out[1].start as usize..out[1].end as usize],
            "PogChamp"
        );
    }

    #[test]
    fn substring_does_not_match() {
        let idx = EmoteIndex::new();
        idx.load([meta("Kappa", Provider::Twitch)]);

        let mut out = Vec::new();
        idx.scan_into("xKappax", &mut out);
        assert!(out.is_empty(), "substring should not match: {out:?}");

        idx.scan_into("Kappa!", &mut out);
        assert!(out.is_empty(), "punctuation-adjacent must not match");
    }

    #[test]
    fn longest_match_wins_on_overlap() {
        let idx = EmoteIndex::new();
        idx.load([
            meta("Kappa", Provider::Twitch),
            meta("KappaHD", Provider::Twitch),
        ]);

        let mut out = Vec::new();
        idx.scan_into("KappaHD rules", &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].emote.code.as_ref(), "KappaHD");
    }

    #[test]
    fn later_load_overwrites_earlier_by_code() {
        let idx = EmoteIndex::new();
        idx.load([
            meta("PogU", Provider::Bttv),
            meta("PogU", Provider::SevenTv),
        ]);
        let hit = idx.lookup("PogU").unwrap();
        assert_eq!(hit.provider, Provider::SevenTv);
    }

    #[test]
    fn reload_swaps_cleanly() {
        let idx = EmoteIndex::new();
        idx.load([meta("Kappa", Provider::Twitch)]);
        assert!(idx.lookup("Kappa").is_some());

        idx.load([meta("Pepega", Provider::SevenTv)]);
        assert!(idx.lookup("Kappa").is_none());
        assert!(idx.lookup("Pepega").is_some());
        assert_eq!(idx.len(), 1);
    }

    #[test]
    fn scan_is_case_sensitive() {
        let idx = EmoteIndex::new();
        idx.load([meta("Kappa", Provider::Twitch)]);

        let mut out = Vec::new();
        idx.scan_into("kappa KAPPA Kappa", &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].start, 12);
    }

    #[test]
    fn scan_preserves_order() {
        let idx = EmoteIndex::new();
        idx.load([
            meta("A", Provider::Twitch),
            meta("BB", Provider::Twitch),
            meta("CCC", Provider::Twitch),
        ]);

        let mut out = Vec::new();
        idx.scan_into("CCC A BB A", &mut out);
        let codes: Vec<&str> = out.iter().map(|s| s.emote.code.as_ref()).collect();
        assert_eq!(codes, ["CCC", "A", "BB", "A"]);
    }

    #[test]
    fn scan_reuses_buffer() {
        let idx = EmoteIndex::new();
        idx.load([meta("Kappa", Provider::Twitch)]);

        let mut out = Vec::with_capacity(4);
        let cap_before = out.capacity();
        idx.scan_into("Kappa Kappa Kappa", &mut out);
        assert_eq!(out.len(), 3);
        out.clear();
        idx.scan_into("Kappa", &mut out);
        assert_eq!(out.len(), 1);
        assert!(out.capacity() >= cap_before);
    }

    #[test]
    fn tab_and_newline_are_boundaries() {
        let idx = EmoteIndex::new();
        idx.load([meta("Kappa", Provider::Twitch)]);

        let mut out = Vec::new();
        idx.scan_into("hi\tKappa\nbye", &mut out);
        assert_eq!(out.len(), 1, "tab+newline should delimit emotes");
        assert_eq!(out[0].start, 3);
        assert_eq!(out[0].end, 8);
    }

    #[test]
    fn empty_code_is_dropped() {
        let idx = EmoteIndex::new();
        idx.load([meta("", Provider::Twitch), meta("Kappa", Provider::Twitch)]);
        assert_eq!(idx.len(), 1);
        assert!(idx.lookup("Kappa").is_some());
    }

    #[test]
    fn only_empty_codes_leaves_index_usable() {
        let idx = EmoteIndex::new();
        idx.load([meta("", Provider::Twitch)]);
        assert!(idx.is_empty());
        let mut out = Vec::new();
        idx.scan_into("hello world", &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn meta_deserializes_from_sidecar_json() {
        // Matches the shape sidecar/internal/emotes writes.
        let raw = r#"{
            "id": "60a",
            "code": "PepegaAim",
            "provider": "7tv",
            "url_1x": "https://cdn.7tv.app/emote/60a/1x.webp",
            "url_2x": "https://cdn.7tv.app/emote/60a/2x.webp",
            "url_4x": "https://cdn.7tv.app/emote/60a/4x.webp",
            "width": 32,
            "height": 32,
            "animated": true,
            "zero_width": true
        }"#;
        let got: EmoteMeta = serde_json::from_str(raw).unwrap();
        assert_eq!(got.code.as_ref(), "PepegaAim");
        assert_eq!(got.provider, Provider::SevenTv);
        assert!(got.animated && got.zero_width);
        assert_eq!(got.width, 32);
    }

    #[test]
    fn bundle_deserializes_from_sidecar_json() {
        // Matches the on-wire shape emitted by sidecar FetchAndNotifyEmotes.
        // Extra fields (badges, errors) are tolerated via serde's default
        // "ignore unknown" behaviour.
        let raw = r#"{
            "twitch_global_emotes": {"provider":"twitch","scope":"global","emotes":[
                {"id":"1","code":"Kappa","provider":"twitch","url_1x":"https://t/1"}
            ]},
            "twitch_channel_emotes": {"provider":"twitch","scope":"channel","emotes":[]},
            "twitch_global_badges": {"scope":"global","badges":[]},
            "twitch_channel_badges": {"scope":"channel","badges":[]},
            "seventv_global": {"provider":"7tv","scope":"global","emotes":[
                {"id":"2","code":"PepegaAim","provider":"7tv","url_1x":"https://s/2"}
            ]},
            "seventv_channel": {"provider":"7tv","scope":"channel","emotes":[]},
            "bttv_global": {"provider":"bttv","scope":"global","emotes":[]},
            "bttv_channel": {"provider":"bttv","scope":"channel","emotes":[]},
            "ffz_global": {"provider":"ffz","scope":"global","emotes":[]},
            "ffz_channel": {"provider":"ffz","scope":"channel","emotes":[]},
            "errors": []
        }"#;
        let b: EmoteBundle = serde_json::from_str(raw).unwrap();
        assert_eq!(b.total_emotes(), 2);
        assert_eq!(b.twitch_global_emotes.emotes[0].code.as_ref(), "Kappa");
        assert_eq!(b.seventv_global.emotes[0].provider, Provider::SevenTv);
    }

    #[test]
    fn load_bundle_channel_overrides_global() {
        let idx = EmoteIndex::new();
        let mut bundle = EmoteBundle::default();
        bundle
            .twitch_global_emotes
            .emotes
            .push(meta("Kappa", Provider::Twitch));
        bundle
            .seventv_channel
            .emotes
            .push(meta("Kappa", Provider::SevenTv));

        idx.load_bundle(bundle);

        // Channel-scoped 7TV overrides the global Twitch emote with the same code.
        let hit = idx.lookup("Kappa").unwrap();
        assert_eq!(hit.provider, Provider::SevenTv);
    }

    #[test]
    fn load_bundle_third_party_overrides_twitch_in_same_scope() {
        let idx = EmoteIndex::new();
        let mut bundle = EmoteBundle::default();
        bundle
            .twitch_global_emotes
            .emotes
            .push(meta("PogChamp", Provider::Twitch));
        bundle
            .seventv_global
            .emotes
            .push(meta("PogChamp", Provider::SevenTv));

        idx.load_bundle(bundle);

        let hit = idx.lookup("PogChamp").unwrap();
        assert_eq!(hit.provider, Provider::SevenTv);
    }

    #[test]
    fn load_bundle_empty_is_safe() {
        let idx = EmoteIndex::new();
        idx.load_bundle(EmoteBundle::default());
        assert!(idx.is_empty());
    }
}
