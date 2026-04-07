# Contributing

Thanks for your interest in contributing to Prismoid.

## Getting Started

1. Fork the repo
2. Clone your fork
3. Install prerequisites: Rust toolchain, Go 1.26+, Node.js 20+, bun
4. Run `bun install` to install frontend dependencies and set up git hooks (lefthook)
5. Run `cargo tauri dev` to start the app in development mode

## Before You Code

- Check existing issues and PRs to avoid duplicate work
- For non-trivial changes, open an issue first to discuss the approach
- Read the [`docs/`](docs/) folder, especially [`adr.md`](docs/adr.md) for decisions that are locked

## Code Standards

- **Rust**: `cargo fmt` and `cargo clippy -- -D warnings` must pass. No warnings
- **Go**: `gofmt` and `go vet ./...` must pass
- **TypeScript**: ESLint and Prettier must pass. `tsc --noEmit` must pass
- Pre-commit hooks enforce formatting and linting automatically via lefthook

## Pull Requests

- Keep PRs focused. One concern per PR
- Write a clear description of what changed and why
- All CI checks must pass before review
- Squash merge into main (enforced by branch rules)

## Commit Messages

Use conventional commits:

```
feat: emote picker fuzzy search
fix: twitch reconnect dropping first message after resume
refactor: extract platform adapter interface
chore: update deps
```

## Testing

- Add tests for new functionality
- Don't break existing tests
- See [`docs/testing.md`](docs/testing.md) for the testing strategy and what to test

## Architecture

The codebase has three language boundaries with clear responsibilities. See [`docs/architecture.md`](docs/architecture.md).

- Don't put network I/O in Rust (that's Go's job)
- Don't put message processing in Go (that's Rust's job)
- Don't put business logic in the frontend (that's Rust's job)

## Contributor License Agreement

By submitting a PR, you agree that your contributions are licensed under the same [GPL-3.0 license](LICENSE) as the project, and you grant the project maintainers the right to relicense your contributions in the future.
