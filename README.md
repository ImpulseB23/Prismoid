# Prismoid

Unified live chat for streamers. Merges Twitch, YouTube, and Kick chat into a single window with cross-platform moderation and universal emote rendering.

## What it does

- One chat feed from all platforms. Messages from Twitch and YouTube (Kick later) appear in a single unified stream
- Mod from one place. Ban, timeout, and delete messages regardless of which platform they came from
- Emotes everywhere. 7TV, BTTV, FFZ, and native platform emotes render in all chats, including YouTube
- OBS overlay. Browser source URL that renders clean unified chat with no platform indicators
- No cloud backend. The app talks directly to platform APIs from your machine. No subscription required

## Stack

- **Rust** (Tauri 2) - desktop shell, message processing, emote scanning
- **Go** (sidecar) - network connections, OAuth, platform APIs
- **TypeScript** (SolidJS) - frontend UI, virtual scrolling, emote rendering

## Development

Prerequisites: Rust toolchain, Go 1.26+, Node.js 20+, bun

```bash
bun install
cargo tauri dev
```

## Documentation

See [`docs/`](docs/) for architecture, platform API details, performance requirements, and decision records.

## License

[GPL-3.0](LICENSE)
