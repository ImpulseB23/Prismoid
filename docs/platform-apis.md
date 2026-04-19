# Platform APIs

## Twitch

### Authentication

OAuth 2.0 Authorization Code flow.

Required scopes:

- `user:read:chat` - read chat messages via EventSub
- `user:write:chat` - send chat messages
- `moderator:manage:banned_users` - ban, timeout, unban
- `moderator:manage:chat_messages` - delete messages

Token storage: OS keychain (never plaintext).
Token refresh: proactive, 5 minutes before expiry.

### Chat (read)

EventSub WebSocket, `channel.chat.message` subscription. This is the current API (not legacy IRC).

Connect to `wss://eventsub.wss.twitch.tv/ws`. On welcome message, subscribe via Helix `POST /eventsub/subscriptions`.

EventSub payloads include emote position data natively.

### Chat (write)

Helix `POST /chat/messages` with `broadcaster_id`, `sender_id`, and `message`.

### Moderation

| Action          | Endpoint                                          |
| --------------- | ------------------------------------------------- |
| Delete message  | `DELETE /moderation/chat` with `message_id`       |
| Timeout         | `POST /moderation/bans` with `duration` (seconds) |
| Ban (permanent) | `POST /moderation/bans` without `duration`        |
| Unban           | `DELETE /moderation/bans`                         |

### Badges

Helix `GET /chat/badges` (global) and `GET /chat/badges?broadcaster_id=` (channel).

### Rate Limits

Twitch Helix: 800 points per minute. Most endpoints cost 1 point. Token bucket rate limiter in Go.

---

## YouTube

### Authentication

Google OAuth 2.0. YouTube Live Streaming API scope. Single app-level Google Cloud project for all users (users never touch an API key).

### Chat (read)

gRPC `liveChatMessages.streamList` - server-streaming RPC. Not REST polling. This keeps quota usage low and latency minimal.

The Go sidecar maintains the gRPC stream and writes raw protobuf bytes to the ring buffer.

### Chat (write)

REST `POST /youtube/v3/liveChat/messages` with `liveChatId` and message text.

### Moderation

| Action           | Endpoint                                                   |
| ---------------- | ---------------------------------------------------------- |
| Delete message   | `DELETE /youtube/v3/liveChat/messages`                     |
| Ban (temporary)  | `POST /youtube/v3/liveChat/bans` with `banDurationSeconds` |
| Ban (permanent)  | `POST /youtube/v3/liveChat/bans` without duration          |
| Unban            | `DELETE /youtube/v3/liveChat/bans`                         |
| Add moderator    | `POST /youtube/v3/liveChat/moderators`                     |
| Remove moderator | `DELETE /youtube/v3/liveChat/moderators`                   |

### Quota

YouTube Data API v3 has a daily quota (default 10,000 units). gRPC streaming keeps read costs near zero. Write operations (send message, mod actions) cost quota units. Request increases from Google as user count grows.

---

## Kick

### Authentication

OAuth 2.1 Authorization Code + PKCE via `id.kick.com`. Kick launched an official public API at `docs.kick.com` in 2025.

Required scopes:

- `chat:write` - send chat messages and allow bots to post
- `events:subscribe` - subscribe to channel events (chat, follows, subs)
- `moderation:ban` - ban/timeout/unban users
- `moderation:chat_message:manage` - delete chat messages
- `channel:read` - read channel information
- `user:read` - read user information

Token storage: OS keychain, same as Twitch (one JSON blob per account).
Token refresh: proactive, 5 minutes before expiry. Token endpoint: `POST https://id.kick.com/oauth/token` with `grant_type=refresh_token`.

### Chat (read)

Pusher WebSocket, not the official webhook API. The official API delivers chat events via webhooks (`POST` to a public URL), which a desktop app cannot receive without a tunnel. Pusher is what Kick's own web client uses and is the standard approach for all third-party Kick clients.

Connect to `wss://ws-us2.pusher.com/app/32cbd69e4b950bf97679?protocol=7&client=js&version=8.4.0-rc2&flash=false`. Subscribe to `chatroom.{chatroom_id}` channel. Messages arrive as Pusher `ChatMessageEvent` events containing sender identity (username, color, badges), message content, emote positions, and reply context.

Chatroom ID is looked up from the channel slug via `GET https://api.kick.com/public/v1/channels?slug={slug}`.

Kick connection failures must never affect Twitch or YouTube connections. Separate goroutine, separate error handling, separate reconnection logic.

### Chat (write)

Official API `POST https://api.kick.com/public/v1/chat` with `broadcaster_user_id`, `content`, and `type` (`"user"` or `"bot"`). Requires `chat:write` scope.

### Moderation

| Action          | Endpoint                                                    |
| --------------- | ----------------------------------------------------------- |
| Delete message  | `DELETE /public/v1/chat/{message_id}`                       |
| Timeout         | `POST /public/v1/moderation/bans` with `duration` (minutes) |
| Ban (permanent) | `POST /public/v1/moderation/bans` without `duration`        |
| Unban           | `DELETE /public/v1/moderation/bans`                         |

Duration range for timeouts: 1 to 10,080 minutes (7 days).

### Rate Limits

Official API rate limits TBD (not yet documented). Token bucket rate limiter in Go, same pattern as Twitch.

---

## Third-Party Emotes

All emote providers are fetched on channel join and compiled into a single aho-corasick automaton per channel.

### 7TV

- Global emotes: `GET https://7tv.io/v3/emote-sets/global`
- Channel emotes: `GET https://7tv.io/v3/users/twitch/{user_id}`
- Image format: WebP, AVIF
- CDN: `https://cdn.7tv.app/emote/{id}/{size}.webp`

### BTTV

- Global emotes: `GET https://api.betterttv.net/3/cached/emotes/global`
- Channel emotes: `GET https://api.betterttv.net/3/cached/users/twitch/{user_id}`
- Image format: GIF, PNG
- CDN: `https://cdn.betterttv.net/emote/{id}/{size}`

### FFZ

- Global emotes: `GET https://api.frankerfacez.com/v1/set/global`
- Channel emotes: `GET https://api.frankerfacez.com/v1/room/id/{user_id}`
- Image format: PNG
- CDN: URLs in API response

### Emote Processing

1. On channel join, fetch all provider emotes concurrently
2. Compile all emote codes into a single aho-corasick automaton
3. Store emote metadata in Rust `Vec<EmoteEntry>` (flat, `#[repr(C)]`, cache-line friendly)
4. On channel switch, build new automaton on background thread, swap atomically via `ArcSwap`
5. Emote images lazy-loaded on viewport entry, cached in memory after first load
6. Filesystem cache at `~/.cache/prismoid/emotes/`, LRU eviction at 500MB
