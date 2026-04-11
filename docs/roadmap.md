# Roadmap

Phase plan for reaching a shippable v0.1.0 and beyond. Defines what each phase contains, in what order, and what closes it.

The design docs describe the **full** architecture of prismoid as a single target — this file exists to sequence the work that implements that architecture. Each phase cites the design docs it draws from rather than duplicating specification.

## Principles

- **One platform first, then scale, then more platforms.** Adding YouTube before virtual scrolling or OAuth just multiplies the surface of unsolved problems.
- **Bottom-up.** No UI element is built before the data flowing into it exists.
- **Locked decisions from `docs/adr.md` are not revisited** unless an ADR is formally revised (see ADR 18 for the precedent).
- **Exit criteria are concrete and testable.** "Done when this specific thing works end-to-end," never "done when it feels right."

## Phase 0 — Foundation

**Status:** Done (2026-04-11).

**Goal:** Prove the architecture end-to-end with one platform, one message, one render cycle.

**Deliverables:**

- SPSC shared memory ring buffer between the Go sidecar and Rust host, with handle inheritance via `CreateProcess` and stdio bootstrap (`docs/architecture.md` §IPC, ADR 18).
- Tauri sidecar lifecycle: spawn, heartbeat stream, control-plane commands over stdio (`docs/architecture.md` §Sidecar Lifecycle).
- Twitch EventSub WebSocket client in the sidecar, serialized through a channel-based writer goroutine into the ring (`docs/platform-apis.md` §Twitch).
- Rust drain loop parsing raw envelopes into `UnifiedMessage` with `catch_unwind` guards (`docs/stability.md` §Rust Panic Handling).
- SolidJS frontend with plain-TS ring buffer + per-frame viewport signal (ADR 21, `docs/frontend.md` §State Management).
- Kernel event signal wakeups replacing polled drain for low-latency IPC.
- Dev-only `.env.local` support for Phase 0 credentials.

**Exit criterion:** A real `channel.chat.message` notification from Twitch EventSub renders in the Solid chat window of `cargo tauri dev`. Verified.

**Out of scope:** Anything listed in later phases.

## Phase 1 — Stability & core UX

**Goal:** Make the app daily-usable on a single platform (Twitch) with correct secret handling, connection resilience, and enough UI to be recognizable as a chat client.

**Deliverables:**

- Sidecar respawn on 3 missed heartbeats (`docs/stability.md` §Sidecar Health). Right now heartbeats flow but nothing monitors them.
- True drop-oldest ring buffer backpressure (`docs/architecture.md` §IPC Ring Buffer). Currently drop-newest; diverges from the spec.
- Twitch OAuth Authorization Code + PKCE flow with OS keychain token storage via the `keyring` crate (`docs/platform-apis.md` §Twitch Authentication). Replaces the Phase 0 env-var creds path.
- Proactive token refresh 5 minutes before expiry (`docs/platform-apis.md` §Twitch).
- UI chrome: header bar (channel name, connection status, platform indicator), timestamps, mod/sub/broadcaster badge chips, username color, a dark theme via CSS custom properties (`docs/frontend.md` §Theming, §Optimistic UI).
- Mod actions: ban, timeout, delete, unban with optimistic UI (`docs/moderation.md`, `docs/frontend.md` §Optimistic UI §Mod Actions).
- Send-message input field with `user:write:chat` scope.
- Basic settings surface (account management, theme toggle).

**Exit criterion:** A streamer can sign in with Twitch, read and moderate their own chat, send messages, and keep the app running across a multi-hour stream with auto-reconnect surviving network blips. Non-developers can run the dev build without editing any files.

## Phase 2 — Scale & polish

**Goal:** Hit the performance targets in `docs/performance.md` so Phase 1's UX holds at real volume.

**Deliverables:**

- Pretext text measurement integration (ADR 13, `docs/frontend.md` §Virtual Scrolling §Layout Computation).
- DOM virtual scrolling with ~80 live nodes max, GPU-composited transform scrolling (`docs/frontend.md` §DOM Management).
- Emote fetching for 7TV, BTTV, FFZ with the token bucket rate limiter (`docs/platform-apis.md` §Third-Party Emotes, ADR 27).
- aho-corasick emote scanner with flat `Vec<EmoteEntry>`, zero-alloc hot path, double-buffered automaton via `ArcSwap` (ADR 17, `docs/performance.md` §Rust Emote Table).
- `OffscreenCanvas`-backed animated emotes on a shared worker timer (ADR 15, `docs/frontend.md` §Animated Emotes).
- Emote picker UI: search, recents/favorites, per-provider tabs (ADR 8).
- SQLite-backed emote cache with LRU eviction at 500 MB (ADR 24, 26, 32).
- Schema migrations framework (ADR 33).
- Zero-alloc drain path (PRI-8) — benchmark-driven, not speculative.

**Exit criterion:** Sustained 10k msg/sec through the full pipeline on a single Twitch channel, under 80 MB resident memory total (`docs/performance.md` §Memory Budget), with emotes rendering correctly including 7TV/BTTV/FFZ overlays on YouTube messages (once Phase 3 lands those).

## Phase 3 — YouTube

**Goal:** Second platform. Delivers the core unique value of prismoid ("unified chat").

**Deliverables:**

- YouTube gRPC `liveChatMessages.streamList` client in the sidecar (`docs/platform-apis.md` §YouTube, ADR 12).
- Google OAuth 2.0 with YouTube Live Streaming API scope (`docs/platform-apis.md` §YouTube Authentication).
- YouTube write path: send message, mod actions (`docs/platform-apis.md` §YouTube Chat/Moderation).
- Hybrid timestamp ordering (arrival-time default, snap to platform timestamp within 500 ms) in the Rust host (`docs/architecture.md` §Message Ordering).
- Cross-platform identity kept separate (ADR 5).
- YouTube badges rendered inline alongside Twitch badges.

**Exit criterion:** Live Twitch + YouTube feeds interleave correctly in one window; mod actions on a YouTube message hit the YouTube Data API successfully; the unified feed sorts messages consistently under the hybrid rule.

## Phase 4 — Distribution

**Goal:** Ship v0.1.0 to real users.

**Deliverables:**

- OBS overlay: Rust localhost HTTP server (axum or tiny_http) serving a browser-source page that subscribes to the same message stream, platform indicators stripped (ADR 7, ADR 16, `docs/frontend.md` §OBS Overlay).
- Tauri auto-updater wired to GitHub Releases (stable + beta channels) (`docs/release-strategy.md` §Auto-Updater Channels).
- System tray with quick actions, minimize-to-tray lifecycle (`docs/architecture.md` §Sidecar Lifecycle).
- Anonymous opt-in telemetry: crash reports + usage stats, no message content (ADR 9).
- Code signing per ADR 6: macOS signed, Windows unsigned in v1.
- Multi-platform CI builds for Windows, macOS (both architectures), Linux.
- First draft release → canary → production promotion flow (`docs/release-strategy.md` §Release Flow).

**Exit criterion:** A non-developer on any supported OS can download v0.1.0 from GitHub Releases, install it, sign in, and use it. Auto-updater reliably bumps them to v0.1.1.

## Phase 5 — Kick + extensions

Already partially scoped in `release-strategy.md` (Cargo feature flags `kick = []`, `extensions = []`) and `platform-apis.md` §Kick.

**Goal:** Third platform + opt-in plugin surface.

**Deliverables:**

- Reverse-engineered Pusher WebSocket Kick client in the sidecar, fully isolated so Kick failures can't affect Twitch/YouTube (`docs/stability.md` §Platform Isolation, `docs/platform-apis.md` §Kick).
- Kick auth, mod actions, rate limiting.
- Plugin API for user extensions (scoped in a dedicated ADR when this phase starts).

**Exit criterion:** `kick` Cargo feature flag becomes default-on; Kick connection runs stable across multi-hour streams alongside Twitch + YouTube.

## Phase 6 — Stream management

Already gated in `release-strategy.md` as `stream-mgmt = []`.

**Goal:** Broadcaster-side features beyond chat.

**Deliverables:**

- Go-live / go-offline controls
- Title + category editing
- Stream-key management
- Queue / timer widgets

Scope and exit criterion TBD when the phase starts.

---

## Cross-phase work (continuous)

- Test coverage for every new module (`docs/testing.md`).
- ADR revisions when locked decisions need to change (see ADR 18 for the precedent).
- Follow-up performance tickets driven by benchmarks, never speculation (PRI-8 pattern).
- Security review at each phase boundary.

## Non-goals (explicit)

- Cloud backend. Locked local-first per the architecture brief.
- Multi-account per platform in v1 (ADR 30).
- Whispers in v1 (ADR 4).
- Subscription model for core features.
