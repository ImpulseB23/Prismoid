# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in prismoid, please report it responsibly.

**Email**: security@prismoid.org

Do not open a public issue for security vulnerabilities.

We will acknowledge receipt within 48 hours and provide a timeline for a fix. Once the vulnerability is resolved, we will credit you in the release notes (unless you prefer to remain anonymous).

## Scope

The following are in scope:

- OAuth token handling and storage
- Shared memory IPC between Rust and Go
- Input sanitization and message rendering (XSS)
- Local HTTP server for OBS overlay
- SQLite access patterns
- Auto-updater integrity

## Out of Scope

- Vulnerabilities in upstream platform APIs (Twitch, YouTube, Kick)
- Vulnerabilities in third-party emote providers (7TV, BTTV, FFZ)
- Social engineering
- Denial of service against the local application

## Token Security

- OAuth tokens are stored in the OS keychain (Windows Credential Manager, macOS Keychain, Linux Secret Service)
- Tokens are never written to disk as plaintext
- Tokens are never logged
- Tokens are never sent to any server other than the issuing platform's API
