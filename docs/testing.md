# Testing Strategy

## Layers

| Layer                 | Tool                                | Scope                                                               | Runs                |
| --------------------- | ----------------------------------- | ------------------------------------------------------------------- | ------------------- |
| Rust unit tests       | `cargo test`                        | Message parsing, emote scanning, ring buffer logic, automaton build | Pre-push hook, CI   |
| Go unit tests         | `go test`                           | WebSocket handling, OAuth flow, rate limiter, platform adapters     | Pre-push hook, CI   |
| TypeScript unit tests | Vitest                              | Store logic, ring buffer, utility functions                         | Pre-push hook, CI   |
| Component tests       | Vitest + `@solidjs/testing-library` | Solid components in JSDOM (emote picker, user cards, mod UI)        | CI                  |
| E2E tests             | WebdriverIO + tauri-driver          | Full pipeline: Go sidecar -> Rust -> frontend in real app           | CI (build required) |

## What to Test

### Rust

- Message parsing for each platform (Twitch EventSub JSON, YouTube protobuf)
- Emote scanning: aho-corasick finds correct positions for overlapping, adjacent, and Unicode-adjacent emotes
- Ring buffer: concurrent read/write, overflow behavior, message integrity
- Automaton rebuild: double-buffer swap doesn't drop messages
- Unified message normalization: all platforms produce valid `UnifiedMessage`

### Go

- WebSocket reconnection with exponential backoff + jitter
- OAuth token refresh (proactive timing, failure handling, concurrent refresh prevention)
- Rate limiter: token bucket refill, burst handling, per-platform isolation
- Platform adapters: each adapter produces valid raw bytes for Rust to parse
- Shared memory write: correct framing, doesn't corrupt on partial write

### TypeScript

- Ring buffer: write/read/overflow, index wrapping
- Virtual scroller: viewport calculation, scroll-to-bottom detection, height accumulation
- Emote picker: fuzzy search ranking, category filtering
- Optimistic UI: state transitions for mod actions (pending, success, failure, revert)
- Store modules: signal updates, derived state correctness

### E2E

- App launches, window appears, sidecar starts within 2 seconds
- Connect to a Twitch channel, messages appear in the chat feed
- Send a message, see it appear with optimistic rendering
- Emotes render correctly (static and animated)
- Mod actions work end-to-end (requires test channel with mod permissions)
- Channel switch: emotes update, automaton rebuilds, no stale emotes visible
- Minimize to tray, restore, connections still alive

## What NOT to Test

- Trivial getters/setters
- Framework internals (Solid's reactivity, Tauri's IPC plumbing)
- Platform API behavior (mock at the adapter boundary)
- CSS styling (visual regression testing is out of scope for v1)

## Mocking Strategy

- Platform APIs: mock at the Go adapter boundary. Each adapter has an interface; tests inject a mock that returns fixture payloads
- Tauri IPC: mock the `invoke` function in component tests
- Shared memory: integration tests use a real ring buffer (it's just memory, no external dependency)
- SQLite: use an in-memory SQLite database for tests
- Network: never hit real platform APIs in CI. Use recorded fixtures

## CI Integration

Tests run on every PR via GitHub Actions. The CI workflow gates merge on:

1. `cargo test` passes
2. `cargo clippy -- -D warnings` passes
3. `go test ./...` passes
4. `go vet ./...` passes
5. `bun vitest run` passes
6. `bun tsc --noEmit` passes
7. E2E tests pass (on scheduled CI runs, not every PR, since they require a full build)
