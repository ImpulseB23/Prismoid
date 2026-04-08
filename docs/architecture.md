# Architecture

## Process Model

prismoid runs three processes:

1. **Tauri shell (Rust)** - the host process. Manages the window, system tray, auto-updater, and the message processing pipeline. Owns the hot path: parsing platform payloads, scanning for emotes via aho-corasick, normalizing messages into the unified format, and batching IPC to the frontend.

2. **Go sidecar** - a child process managed by Tauri's sidecar API. Owns all network I/O: Twitch EventSub WebSocket, YouTube gRPC streaming, 7TV/BTTV/FFZ REST APIs, and OAuth token management. Writes raw message bytes to the shared memory ring buffer.

3. **Frontend (TypeScript/SolidJS)** - runs in Tauri's WebView. Renders the unified chat feed via virtual scrolling, handles emote display, mod action UI, settings, and theming.

```
┌──────────────────────────────────────────────┐
│                  Tauri Shell (Rust)           │
│                                              │
│  ┌─────────────┐    ┌─────────────────────┐  │
│  │  Processing  │<-->│  Shared Memory Ring  │  │
│  │  Core (Rust) │    │  Buffer (IPC)        │  │
│  └──────┬──────┘    └────────^────────────┘  │
│         │                    │               │
│         │ Tauri IPC          │               │
│         v                    │               │
│  ┌─────────────┐    ┌───────┴─────────┐     │
│  │  Frontend    │    │  Go Sidecar     │     │
│  │  (SolidJS)   │    │  (Network Layer)│     │
│  └─────────────┘    └─────────────────┘     │
└──────────────────────────────────────────────┘
```

## Data Flow

### Messages (hot path)

```
Platform API --> Go sidecar --> ring buffer --> Rust processing --> Tauri IPC --> frontend
```

1. Go receives raw platform messages (EventSub JSON, gRPC protobuf, etc.)
2. Go writes raw bytes into the shared memory ring buffer (write side)
3. Rust reads from the ring buffer, parses the payload, scans for emotes using the aho-corasick automaton, and normalizes into `UnifiedMessage`
4. Rust batches all messages that arrived within one 16ms frame into a single IPC payload
5. Frontend receives the batch, writes to the message ring buffer (plain TypeScript, outside Solid reactivity), and updates the viewport signal once per frame

### Mod Actions (reverse path)

```
Frontend --> Tauri IPC --> Rust --> ring buffer / command channel --> Go --> platform API
```

1. Frontend updates UI optimistically (strikethrough, gray out) immediately on click
2. Tauri IPC sends the action to Rust
3. Rust routes by platform, sends to Go via the ring buffer or a dedicated command channel
4. Go dispatches to the correct platform API (Twitch Helix, YouTube REST)
5. On confirmation, no UI change needed (already updated). On failure, Rust notifies frontend to revert

## IPC: Shared Memory Ring Buffer

The ring buffer is the primary data channel between Go and Rust. Chosen over stdio pipes and unix domain sockets for zero-copy, zero-serialization throughput at 10,000+ msg/sec.

- **Size**: 4MB fixed allocation (~8,000 messages of headroom at ~500 bytes avg)
- **Backpressure**: if Go writes faster than Rust reads, oldest unread messages are dropped. Acceptable at extreme volume since the frontend is already frame-dropping
- **Cross-platform**: `shared_memory` crate on the Rust side (abstracts POSIX `shm_open` and Windows `CreateFileMapping`). `golang.org/x/sys/windows` for `CreateFileMapping`/`MapViewOfFile` on Windows, `syscall.Mmap` on POSIX
- **Control plane**: Tauri's sidecar stdio channel handles lightweight commands (heartbeats, health checks, channel switch, automaton rebuild triggers). Heavy data never touches stdio

## Sidecar Lifecycle

Tauri spawns the Go sidecar on startup. Rust monitors it via heartbeats over the stdio control plane.

- Heartbeat interval: 1 second
- Missing 3 consecutive heartbeats triggers a respawn
- On crash, Rust respawns the sidecar within seconds. A few messages may be lost during the gap. The app never dies
- On graceful shutdown (window close to tray), the sidecar stays alive. Connections stay warm. 30-minute idle timeout before flushing caches and dropping connections

## Unified Message Format

Every platform adapter in Go produces raw bytes. Rust parses them into this struct:

```rust
struct UnifiedMessage {
    id: String,
    platform: Platform,          // Twitch | YouTube | Kick
    timestamp: i64,              // platform timestamp (ms)
    arrival_time: i64,           // local arrival time (ms)
    username: String,
    display_name: String,
    platform_user_id: String,
    message_text: String,
    emote_positions: Vec<EmotePosition>,
    badges: Vec<Badge>,
    is_mod: bool,
    is_subscriber: bool,
    is_broadcaster: bool,
    reply_to: Option<String>,
    platform_metadata: PlatformMeta,
}
```

The frontend only knows about `UnifiedMessage`. It never sees platform-specific payloads.

## Message Ordering

Hybrid approach: arrival time by default. If a message's platform timestamp is within 500ms of its arrival time, snap to platform timestamp. This prevents visual reordering while keeping perceived real-time flow.

## Cross-Platform Identity

v1: separate. The same person on Twitch and YouTube appears as two distinct chatters with platform badges. No identity linking, no heuristic matching.
