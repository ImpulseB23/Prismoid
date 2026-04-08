# Copilot Instructions

## Project Overview

Prismoid is a native desktop app (Tauri 2) that merges live chat from Twitch, YouTube, and Kick into a single window. Three-language stack: Rust, Go, TypeScript.

## Architecture Boundaries

- **Rust** (apps/desktop/src-tauri/): Tauri shell, message processing hot path, emote scanning (aho-corasick), SQLite caching, sidecar lifecycle management. All message parsing and normalization happens here.
- **Go** (apps/desktop/src-sidecar/): Network I/O only. WebSocket connections (Twitch EventSub, Kick Pusher), YouTube gRPC streaming, 7TV/BTTV/FFZ REST APIs, OAuth token management. Writes raw bytes to shared memory ring buffer.
- **TypeScript** (apps/desktop/src/): SolidJS frontend. Virtual scrolling with Pretext, emote rendering, mod action UI, settings, theming.

Never put network I/O in Rust. Never put message processing in Go. Never put business logic in the frontend.

## Performance Constraints

This app must handle 10,000+ messages per second at peak and run alongside a game and OBS.

- Zero heap allocation per message on the Rust hot path
- Message buffer in the frontend is a plain TypeScript ring buffer, NOT a Solid store/signal
- One Solid signal update per frame (viewport position), never one per message
- Max ~80 DOM nodes in the chat list at any time (virtual scrolling)
- No `box-shadow`, complex `border-radius`, or `filter` on message elements
- Transitions use `transform` and `opacity` only

## IPC

Go and Rust communicate via a shared memory ring buffer (4MB, zero-copy). Tauri sidecar stdio is the control plane (heartbeats, commands). Heavy message data never goes through stdio.

## Code Style

- Conventional commits: `feat:`, `fix:`, `chore:`, `refactor:`
- Rust: follow `cargo clippy` and `cargo fmt` defaults
- Go: follow `gofmt` and `go vet`
- TypeScript: ESLint + Prettier, strict mode
- Minimal comments. Only where intent is non-obvious
- No TODO/FIXME/HACK comments. Implement it or don't
- No placeholder code or stub functions

## Key Libraries

- Rust: tauri 2, shared_memory, aho-corasick, arc-swap, rusqlite, tracing
- Go: google.golang.org/grpc, golang.org/x/sys, zerolog
- TypeScript: solid-js, @chenglou/pretext, vite

## Decisions

See docs/adr.md for locked architecture decisions. Do not contradict them.
