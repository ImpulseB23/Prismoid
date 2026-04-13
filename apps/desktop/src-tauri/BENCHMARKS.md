# Ring buffer drain + parse benchmarks

Microbenchmarks for the host's hot path. See
[`benches/drain_throughput.rs`](benches/drain_throughput.rs) and PRI-15.

## Running

```sh
# From apps/desktop/src-tauri
cargo bench --features __bench
```

The `__bench` feature exposes internal writer + parser helpers to the
bench binary only. It must be explicitly enabled; `cargo bench` without
it will fail at link time with a missing-symbols error. The feature
flag never ships in release builds.

Criterion writes HTML reports to `target/criterion/<group>/<param>/report/`.

## Groups

- **`drain_only`** — `RingBufReader::drain()` on a ring pre-filled with
  N copies of a representative Twitch `channel.chat.message` envelope
  (~1.1 KiB each). Measures the cost of walking the ring + copying
  bytes out into `Vec<Vec<u8>>`.
- **`parse_only`** — `host::parse_batch` on a pre-built
  `Vec<Vec<u8>>`. Measures `serde_json::from_slice` + timestamp parse
  - flag-derivation, isolated from the ring.
- **`drain_and_parse`** — the full supervisor-hot-loop shape:
  pre-fill ring, drain, parse, emit-ready batch.

Each group sweeps N ∈ {10, 100, 1000, 10000} and reports throughput in
elements/sec.

## Baseline

Captured 2026-04-13 via `cargo bench --features __bench -- --warm-up-time 1 --measurement-time 3 --sample-size 30`.
Reference machine: Windows 11, Ryzen desktop. Median times. Lower is better.

| Group             | N=10    | N=100    | N=1000   | N=10000  |
| ----------------- | ------- | -------- | -------- | -------- |
| `drain_only`      | 16.7 µs | 16.9 µs  | 205.9 µs | 4.45 ms  |
| `parse_only`      | 34.8 µs | 489.9 µs | 5.40 ms  | 49.85 ms |
| `drain_and_parse` | 73.7 µs | 590.6 µs | 5.64 ms  | 55.84 ms |

### Reading the numbers

- Combined `drain_and_parse` at N=10000 = **55.8 ms**, i.e. ~179 k msg/sec
  per drain-and-parse cycle — roughly **18× over** the 10k/sec peak target
  from `docs/performance.md`. Headroom is comfortable.
- `parse_only` dominates: ~5 µs per message, mostly `serde_json::from_slice`
  doing owned-`String` field allocations. This is PRI-8's main lever
  (simd-json or sonic-rs would plausibly cut it by 3-5×).
- `drain_only` is ~440 ns/message at N=10000, almost all of which is the
  per-message `Vec<u8>` allocation in `drain() -> Vec<Vec<u8>>`. Replacing
  that with a callback-style `drain_with<F>` is PRI-8's secondary lever.
- Small-N measurements (N=10) are dominated by fixed per-iteration
  overhead (64 MiB `CreateFileMapping` first-page-touch, criterion
  instrumentation). Compare relative deltas at N=1000+ for meaningful
  signal.

## Not running in CI

CI intentionally does not run benches (too slow, too noisy, hardware
variance dominates signal). Treat the numbers in this file as a local
baseline against which PRI-8's before/after delta is the only thing
that matters — the absolute values are not a contract.
