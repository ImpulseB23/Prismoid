<p align="center">
  <picture>
    <source srcset="assets/icon-white.svg" media="(prefers-color-scheme: dark)" />
    <img src="assets/icon.svg" alt="prismoid" width="120" />
  </picture>
</p>

<h1 align="center">prismoid</h1>

<p align="center">
  unified live chat for streamers
</p>

<p align="center">
  <a href="https://prismoid.org">website</a> &middot;
  <a href="https://github.com/ImpulseB23/Prismoid/releases">download</a> &middot;
  <a href="CONTRIBUTING.md">contribute</a> &middot;
  <a href="docs/">docs</a>
</p>

---

merges Twitch, YouTube, and Kick chat into a single window with cross-platform moderation and universal emote rendering.

- **one feed** from all platforms in a single stream
- **mod from one place** - ban, timeout, delete regardless of source platform
- **emotes everywhere** - 7TV, BTTV, FFZ render in all chats, including YouTube
- **OBS overlay** - browser source for clean unified chat on stream
- **no cloud backend** - talks directly to platform APIs from your machine

## stack

|                          |                                                          |
| ------------------------ | -------------------------------------------------------- |
| **Rust** (Tauri 2)       | desktop shell, message processing, emote scanning        |
| **Go** (sidecar)         | network I/O, WebSocket connections, OAuth, platform APIs |
| **TypeScript** (SolidJS) | frontend UI, virtual scrolling, emote rendering          |

## development

prerequisites: Rust toolchain, Go 1.26+, Node.js 20+, bun

```bash
cd apps/desktop
bun install
cargo tauri dev
```

## screenshots

> coming soon

## docs

see [`docs/`](docs/) for architecture, platform API details, performance requirements, and decision records.

## license

[GPL-3.0](LICENSE)
