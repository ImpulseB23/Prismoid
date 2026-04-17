//! Baseline benchmarks for the host's ring-buffer drain + parse hot path.
//!
//! Enabled only under `cargo bench --features __bench` (see `Cargo.toml`).
//! Four groups:
//! - `drain_only`    — `RingBufReader::drain()` on a pre-filled ring
//! - `parse_only`    — `host::parse_batch` on a pre-built `Vec<Vec<u8>>`
//! - `drain_and_parse` — the full hot-loop shape the supervisor runs
//!
//! Parse and drain+parse are swept across both an empty [`EmoteIndex`]
//! (lower bound, scan short-circuits) and a populated one sized like a
//! real channel join (~500 codes with a handful matching the fixture
//! message), so the numbers bracket the true hot-path cost.
//!
//! See PRI-15 for why this lands first and PRI-8 for what the numbers
//! are meant to gate.

#[cfg(not(windows))]
compile_error!(
    "drain_throughput bench is windows-only: the ring buffer primitive is \
     implemented on windows first (ADR 18)"
);

use criterion::{
    black_box, criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput,
};

use prismoid_lib::emote_index::{EmoteBundle, EmoteIndex, EmoteMeta, EmoteSet, Provider};
use prismoid_lib::ringbuf::RingBufReader;
use prismoid_lib::{parse_batch, UnifiedMessage};

/// Ring capacity large enough to hold the worst-case sweep point
/// (`10_000 * TWITCH_MESSAGE.len()` ≈ 17 MiB) without wrapping during
/// pre-fill, with headroom for the 4-byte length prefix per message
/// and alignment. 64 MiB is cheap in a bench process and removes
/// wrap-around effects as a confounder — under-sizing silently
/// corrupts the ring and collapses measured latency to nanoseconds
/// (drain fast-bails on a corrupt length prefix).
const BENCH_CAPACITY: usize = 64 * 1024 * 1024;

/// The parameter sweep. Covers the full range from a trivial batch up to
/// the 10k/tick figure docs/performance.md calls out as the target.
const SWEEP_SIZES: &[u32] = &[10, 100, 1_000, 10_000];

/// Representative `channel.chat.message` EventSub notification, matching
/// the envelope shape the Twitch WebSocket sends in production. Length
/// is ~1.1 KiB, which lines up with a median real chat message plus the
/// envelope overhead Twitch wraps around it.
const TWITCH_MESSAGE: &[u8] = br##"{
    "metadata": {
        "message_id": "35064eb1-c4a5-5bd0-4a0b-3f3e9e9d5001",
        "message_type": "notification",
        "message_timestamp": "2026-04-12T20:15:32.847Z",
        "subscription_type": "channel.chat.message",
        "subscription_version": "1"
    },
    "payload": {
        "subscription": {
            "id": "abc123-def-456-ghi-789",
            "status": "enabled",
            "type": "channel.chat.message",
            "version": "1",
            "cost": 0,
            "condition": {
                "broadcaster_user_id": "570722168",
                "user_id": "570722168"
            },
            "transport": {
                "method": "websocket",
                "session_id": "AgoQsess123-abc-def-ghi-jkl"
            }
        },
        "event": {
            "broadcaster_user_id": "570722168",
            "broadcaster_user_login": "prismoiddev",
            "broadcaster_user_name": "PrismoidDev",
            "chatter_user_id": "123456789",
            "chatter_user_login": "typical_viewer42",
            "chatter_user_name": "Typical_Viewer42",
            "message_id": "cc106a89-1814-919d-454c-f4f2f970aae7",
            "message": {
                "text": "this is a pretty average length chat message talking about the stream",
                "fragments": [
                    {"type": "text", "text": "this is a pretty average length chat message talking about the stream", "cheermote": null, "emote": null, "mention": null}
                ]
            },
            "color": "#1E90FF",
            "badges": [
                {"set_id": "subscriber", "id": "12", "info": "12"},
                {"set_id": "premium", "id": "1", "info": ""}
            ],
            "message_type": "text"
        }
    }
}"##;

fn drain_only(c: &mut Criterion) {
    let mut group = c.benchmark_group("drain_only");
    for &n in SWEEP_SIZES {
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_batched_ref(
                || {
                    let reader =
                        RingBufReader::create_owner(BENCH_CAPACITY).expect("owner ring for bench");
                    let payloads: Vec<&[u8]> = (0..n).map(|_| TWITCH_MESSAGE).collect();
                    reader.__bench_write(&payloads);
                    reader
                },
                |reader| {
                    black_box(reader.drain());
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn parse_only(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse_only");
    let empty = EmoteIndex::new();
    let populated = populated_index();
    for &n in SWEEP_SIZES {
        let raw: Vec<Vec<u8>> = (0..n).map(|_| TWITCH_MESSAGE.to_vec()).collect();
        let mut batch: Vec<UnifiedMessage> = Vec::with_capacity(n as usize);
        group.throughput(Throughput::Elements(n as u64));
        for (label, idx) in [("empty", &empty), ("populated", &populated)] {
            group.bench_with_input(
                BenchmarkId::new(label, n),
                &(raw.clone(), idx),
                |b, (raw, idx)| {
                    b.iter(|| {
                        batch.clear();
                        parse_batch(raw, &mut batch, idx);
                        black_box(&batch);
                    });
                },
            );
        }
    }
    group.finish();
}

fn drain_and_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("drain_and_parse");
    let empty = EmoteIndex::new();
    let populated = populated_index();
    for &n in SWEEP_SIZES {
        let mut batch: Vec<UnifiedMessage> = Vec::with_capacity(n as usize);
        group.throughput(Throughput::Elements(n as u64));
        for (label, idx) in [("empty", &empty), ("populated", &populated)] {
            group.bench_with_input(BenchmarkId::new(label, n), &n, |b, &n| {
                b.iter_batched_ref(
                    || {
                        let reader = RingBufReader::create_owner(BENCH_CAPACITY)
                            .expect("owner ring for bench");
                        let payloads: Vec<&[u8]> = (0..n).map(|_| TWITCH_MESSAGE).collect();
                        reader.__bench_write(&payloads);
                        reader
                    },
                    |reader| {
                        batch.clear();
                        let raw = reader.drain();
                        parse_batch(&raw, &mut batch, idx);
                        black_box(&batch);
                    },
                    BatchSize::SmallInput,
                );
            });
        }
    }
    group.finish();
}

/// Builds an [`EmoteIndex`] roughly the size of a real channel join:
/// ~500 codes across Twitch, 7TV, BTTV, and FFZ globals + channel sets.
/// A handful of codes deliberately match tokens inside `TWITCH_MESSAGE`
/// ("this", "chat", "message", "stream") so aho-corasick iteration
/// produces both hits and misses instead of a purely cold walk.
fn populated_index() -> EmoteIndex {
    fn meta(code: &str, provider: Provider) -> EmoteMeta {
        EmoteMeta {
            id: code.into(),
            code: code.into(),
            provider,
            url_1x: "https://cdn/1x".into(),
            url_2x: "".into(),
            url_4x: "".into(),
            width: 28,
            height: 28,
            animated: false,
            zero_width: false,
        }
    }

    // Realistic matchers against the fixture message body.
    let matching = [
        "this", "chat", "message", "stream", "the", "about", "length",
    ];
    // Filler to hit ~500 codes total. Keep codes short and camel-case so
    // the aho-corasick automaton resembles a real emote catalog, where
    // most codes are 4-12 chars and begin with a capital letter.
    let filler_roots = [
        "Kappa", "PogU", "OMEGALUL", "Pepe", "Monka", "Kek", "Pog", "LUL", "Sad", "Hype", "Jam",
        "Dance", "Clap", "Wave", "Wink", "Stare", "Cry", "Laugh", "Angy", "Chill", "Cozy", "Doge",
        "Catto", "Birb", "Based", "Cringe", "Copium", "Hopium", "Juicer", "Griddy", "Yeet", "Vibe",
        "Wiggle", "Spin", "Nod", "Shrug", "Facepalm", "Gachi", "EZ", "Bruh",
    ];
    let mut twitch_global = Vec::with_capacity(matching.len() + filler_roots.len() * 3);
    for code in matching {
        twitch_global.push(meta(code, Provider::Twitch));
    }
    for root in filler_roots {
        twitch_global.push(meta(root, Provider::Twitch));
    }
    let seventv_global: Vec<EmoteMeta> = filler_roots
        .iter()
        .flat_map(|r| {
            [
                format!("{r}W"),
                format!("{r}2"),
                format!("{r}X"),
                format!("{r}Jam"),
            ]
        })
        .map(|c| meta(&c, Provider::SevenTv))
        .collect();
    let bttv_global: Vec<EmoteMeta> = filler_roots
        .iter()
        .flat_map(|r| [format!("B{r}"), format!("{r}B")])
        .map(|c| meta(&c, Provider::Bttv))
        .collect();
    let ffz_global: Vec<EmoteMeta> = filler_roots
        .iter()
        .map(|r| meta(&format!("F{r}"), Provider::Ffz))
        .collect();

    let bundle = EmoteBundle {
        twitch_global_emotes: EmoteSet {
            emotes: twitch_global,
        },
        seventv_global: EmoteSet {
            emotes: seventv_global,
        },
        bttv_global: EmoteSet {
            emotes: bttv_global,
        },
        ffz_global: EmoteSet { emotes: ffz_global },
        ..Default::default()
    };
    let idx = EmoteIndex::new();
    idx.load_bundle(bundle);
    idx
}

criterion_group!(benches, drain_only, parse_only, drain_and_parse);
criterion_main!(benches);
