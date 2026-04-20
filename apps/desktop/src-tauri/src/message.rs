//! Unified message type emitted to the frontend, plus parsers that convert
//! raw platform envelopes into it.

use serde::{Deserialize, Serialize};

use crate::emote_index::EmoteSpan;

#[derive(Debug, Clone, Serialize)]
pub enum Platform {
    Twitch,
    YouTube,
    Kick,
}

#[derive(Debug, Clone, Serialize)]
pub struct Badge {
    pub set_id: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnifiedMessage {
    pub id: String,
    pub platform: Platform,
    pub timestamp: i64,
    pub arrival_time: i64,
    /// Effective sort timestamp for unified ordering across platforms.
    /// Set by [`compute_effective_ts`] using the snap rule: trust the
    /// platform-stamped `timestamp` when it agrees with `arrival_time`
    /// within [`SNAP_WINDOW_MS`], else fall back to `arrival_time`. This
    /// keeps cross-platform interleave coherent without ever reordering
    /// already-rendered messages because of vendor clock disagreement.
    pub effective_ts: i64,
    /// Monotonic per-process arrival counter assigned by [`assign_arrival_seqs`].
    /// The frontend uses `(effective_ts, arrival_seq)` as a stable sort
    /// key so two messages with identical effective timestamps never swap
    /// position on re-render.
    pub arrival_seq: u64,
    pub username: String,
    pub display_name: String,
    pub platform_user_id: String,
    pub message_text: String,
    pub badges: Vec<Badge>,
    pub is_mod: bool,
    pub is_subscriber: bool,
    pub is_broadcaster: bool,
    pub color: Option<String>,
    pub reply_to: Option<String>,
    /// Emote matches inside [`message_text`](Self::message_text), populated
    /// by [`crate::emote_index::EmoteIndex::scan_into`] after parsing.
    /// Empty when no index is active or the message has no emotes.
    pub emote_spans: Vec<EmoteSpan>,
}

/// Tolerance window for snapping `effective_ts` to the platform-stamped
/// `timestamp`. Inside this window we trust the platform's clock; outside
/// it we fall back to local arrival time so cross-platform interleave
/// stays coherent. 500 ms is enough to absorb normal vendor clock skew
/// without letting badly delayed messages time-travel up the visible list.
pub const SNAP_WINDOW_MS: i64 = 500;

/// Computes the effective sort timestamp using the snap rule documented
/// on [`UnifiedMessage::effective_ts`]. Pure so the rule can be tested
/// directly without going through a parser.
#[inline]
pub fn compute_effective_ts(timestamp: i64, arrival_time: i64) -> i64 {
    if (timestamp - arrival_time).abs() <= SNAP_WINDOW_MS {
        timestamp
    } else {
        arrival_time
    }
}

/// Stamps every message in `batch` with a monotonic `arrival_seq`,
/// advancing the caller-owned `next_seq` once per message. The drain loop
/// owns the counter so seq is unique for the lifetime of one sidecar run,
/// and the frontend's `(effective_ts, arrival_seq)` sort key remains
/// stable.
pub fn assign_arrival_seqs(batch: &mut [UnifiedMessage], next_seq: &mut u64) {
    for msg in batch.iter_mut() {
        msg.arrival_seq = *next_seq;
        *next_seq = next_seq.wrapping_add(1);
    }
}

#[derive(Debug)]
pub enum ParseError {
    Json(serde_json::Error),
    Timestamp(chrono::ParseError),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(e) => write!(f, "json parse failed: {e}"),
            Self::Timestamp(e) => write!(f, "timestamp parse failed: {e}"),
        }
    }
}

impl std::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(e) => Some(e),
            Self::Timestamp(e) => Some(e),
        }
    }
}

impl From<serde_json::Error> for ParseError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err)
    }
}

// --- Twitch EventSub deserialization types (private, narrow) ---

#[derive(Debug, Deserialize)]
struct TwitchEnvelope {
    metadata: TwitchMetadata,
    #[serde(default)]
    payload: Option<TwitchPayload>,
}

#[derive(Debug, Deserialize)]
struct TwitchMetadata {
    message_type: String,
    message_timestamp: String,
}

#[derive(Debug, Deserialize)]
struct TwitchPayload {
    #[serde(default)]
    subscription: Option<TwitchSubscription>,
    // Event is held as an opaque Value and only deserialized to the concrete
    // chat event type once we have confirmed the subscription type matches.
    // This lets non-chat notifications (channel.follow, etc.) parse cleanly
    // instead of failing on the missing chat-specific fields.
    #[serde(default)]
    event: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct TwitchSubscription {
    #[serde(rename = "type")]
    subscription_type: String,
}

#[derive(Debug, Deserialize)]
struct TwitchChatEvent {
    chatter_user_id: String,
    chatter_user_login: String,
    chatter_user_name: String,
    message_id: String,
    message: TwitchChatMessage,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    badges: Vec<TwitchBadge>,
    #[serde(default)]
    reply: Option<TwitchReply>,
}

#[derive(Debug, Deserialize)]
struct TwitchChatMessage {
    text: String,
}

#[derive(Debug, Deserialize)]
struct TwitchBadge {
    set_id: String,
    id: String,
}

#[derive(Debug, Deserialize)]
struct TwitchReply {
    parent_message_id: String,
}

/// Parses a raw Twitch EventSub envelope into a [`UnifiedMessage`] when the
/// envelope is a `channel.chat.message` notification.
///
/// Returns `Ok(None)` for any other envelope shape — keepalives, reconnects,
/// revocations, notifications for other subscription types, or payloads that
/// are missing the nested `subscription`/`event` fields. Those are not parse
/// errors; the drain loop filters them out silently.
///
/// Defensive parsing per `docs/stability.md`: no `unwrap` on external data,
/// unknown fields are ignored via serde, missing optional fields default.
pub fn parse_twitch_envelope(bytes: &[u8]) -> Result<Option<UnifiedMessage>, ParseError> {
    let envelope: TwitchEnvelope = serde_json::from_slice(bytes)?;

    if envelope.metadata.message_type != "notification" {
        return Ok(None);
    }

    let Some(payload) = envelope.payload else {
        return Ok(None);
    };
    let Some(subscription) = payload.subscription else {
        return Ok(None);
    };
    if subscription.subscription_type != "channel.chat.message" {
        return Ok(None);
    }
    let Some(event_value) = payload.event else {
        return Ok(None);
    };
    let event: TwitchChatEvent = serde_json::from_value(event_value)?;

    let platform_ts = chrono::DateTime::parse_from_rfc3339(&envelope.metadata.message_timestamp)
        .map_err(ParseError::Timestamp)?
        .timestamp_millis();
    let arrival_time = chrono::Utc::now().timestamp_millis();

    let badges: Vec<Badge> = event
        .badges
        .into_iter()
        .map(|b| Badge {
            set_id: b.set_id,
            id: b.id,
        })
        .collect();
    let is_broadcaster = badges.iter().any(|b| b.set_id == "broadcaster");
    let is_mod = is_broadcaster || badges.iter().any(|b| b.set_id == "moderator");
    let is_subscriber = badges
        .iter()
        .any(|b| b.set_id == "subscriber" || b.set_id == "founder");

    Ok(Some(UnifiedMessage {
        id: event.message_id,
        platform: Platform::Twitch,
        timestamp: platform_ts,
        arrival_time,
        effective_ts: 0,
        arrival_seq: 0,
        username: event.chatter_user_login,
        display_name: event.chatter_user_name,
        platform_user_id: event.chatter_user_id,
        message_text: event.message.text,
        badges,
        is_mod,
        is_subscriber,
        is_broadcaster,
        color: event.color,
        reply_to: event.reply.map(|r| r.parent_message_id),
        emote_spans: Vec::new(),
    }))
}

// --- YouTube protojson deserialization types (private, narrow) ---

#[derive(Debug, Deserialize)]
struct YouTubeMessage {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    snippet: Option<YouTubeSnippet>,
    #[serde(default)]
    author_details: Option<YouTubeAuthorDetails>,
}

#[derive(Debug, Deserialize)]
struct YouTubeSnippet {
    #[serde(default, rename = "type")]
    msg_type: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    display_message: Option<String>,
    #[serde(default)]
    text_message_details: Option<YouTubeTextDetails>,
}

#[derive(Debug, Deserialize)]
struct YouTubeTextDetails {
    #[serde(default)]
    message_text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct YouTubeAuthorDetails {
    #[serde(default)]
    channel_id: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    is_chat_owner: Option<bool>,
    #[serde(default)]
    is_chat_moderator: Option<bool>,
    #[serde(default)]
    is_chat_sponsor: Option<bool>,
}

/// Parses a protojson-serialized YouTube `LiveChatMessage` into a
/// [`UnifiedMessage`]. Returns `Ok(None)` for non-text message types
/// (super chats, bans, etc. are future work).
pub fn parse_youtube_message(bytes: &[u8]) -> Result<Option<UnifiedMessage>, ParseError> {
    let msg: YouTubeMessage = serde_json::from_slice(bytes)?;

    let snippet = match msg.snippet {
        Some(s) => s,
        None => return Ok(None),
    };

    // Only handle text messages for now
    let msg_type = snippet.msg_type.as_deref().unwrap_or("");
    if msg_type != "TEXT_MESSAGE_EVENT" {
        return Ok(None);
    }

    let text = match snippet
        .text_message_details
        .and_then(|d| d.message_text)
        .or(snippet.display_message)
        .filter(|s| !s.is_empty())
    {
        Some(t) => t,
        None => return Ok(None),
    };

    let author = match msg.author_details {
        Some(a) => a,
        None => return Ok(None),
    };

    let channel_id = match author.channel_id.filter(|s| !s.is_empty()) {
        Some(c) => c,
        None => return Ok(None),
    };
    let display_name = match author.display_name.filter(|s| !s.is_empty()) {
        Some(d) => d,
        None => return Ok(None),
    };
    let id = match msg.id.filter(|s| !s.is_empty()) {
        Some(i) => i,
        None => return Ok(None),
    };

    let is_broadcaster = author.is_chat_owner.unwrap_or(false);
    let is_mod = is_broadcaster || author.is_chat_moderator.unwrap_or(false);
    let is_subscriber = author.is_chat_sponsor.unwrap_or(false);

    let mut badges = Vec::with_capacity(2);
    if is_broadcaster {
        badges.push(Badge {
            set_id: String::from("youtube/owner"),
            id: String::from("1"),
        });
    } else if author.is_chat_moderator.unwrap_or(false) {
        badges.push(Badge {
            set_id: String::from("youtube/moderator"),
            id: String::from("1"),
        });
    }
    if is_subscriber {
        badges.push(Badge {
            set_id: String::from("youtube/member"),
            id: String::from("1"),
        });
    }

    let color = if is_broadcaster {
        Some(String::from("#ffd600"))
    } else if author.is_chat_moderator.unwrap_or(false) {
        Some(String::from("#5e84f1"))
    } else if is_subscriber {
        Some(String::from("#2ba640"))
    } else {
        None
    };

    let timestamp = match snippet.published_at.as_deref() {
        Some(s) => chrono::DateTime::parse_from_rfc3339(s)
            .map_err(ParseError::Timestamp)?
            .timestamp_millis(),
        None => chrono::Utc::now().timestamp_millis(),
    };

    Ok(Some(UnifiedMessage {
        id,
        platform: Platform::YouTube,
        timestamp,
        arrival_time: chrono::Utc::now().timestamp_millis(),
        effective_ts: 0,
        arrival_seq: 0,
        username: channel_id.clone(),
        display_name,
        platform_user_id: channel_id,
        message_text: text,
        badges,
        is_mod,
        is_subscriber,
        is_broadcaster,
        color,
        reply_to: None,
        emote_spans: Vec::new(),
    }))
}

// --- Kick Pusher deserialization types (private, narrow) ---

#[derive(Debug, Deserialize)]
struct KickPusherEvent {
    event: String,
    #[serde(default)]
    data: Option<String>,
}

#[derive(Debug, Deserialize)]
struct KickChatMessage {
    id: String,
    #[serde(default)]
    #[allow(dead_code)]
    chatroom_id: Option<i64>,
    content: String,
    created_at: String,
    sender: KickSender,
}

#[derive(Debug, Deserialize)]
struct KickSender {
    id: i64,
    username: String,
    #[serde(default)]
    identity: Option<KickIdentity>,
}

#[derive(Debug, Deserialize)]
struct KickIdentity {
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    badges: Vec<KickBadge>,
}

#[derive(Debug, Deserialize)]
struct KickBadge {
    #[serde(rename = "type")]
    badge_type: String,
}

/// Parses a raw Kick Pusher channel event into a [`UnifiedMessage`] when the
/// event is a `ChatMessageEvent`.
///
/// Returns `Ok(None)` for non-chat events. The Pusher `data` field is a
/// double-encoded JSON string containing the actual chat message payload.
pub fn parse_kick_event(bytes: &[u8]) -> Result<Option<UnifiedMessage>, ParseError> {
    let event: KickPusherEvent = serde_json::from_slice(bytes)?;

    if !event.event.contains("ChatMessageEvent") {
        return Ok(None);
    }

    let Some(data_str) = event.data else {
        return Ok(None);
    };

    let msg: KickChatMessage = serde_json::from_str(&data_str)?;

    let platform_ts = chrono::DateTime::parse_from_rfc3339(&msg.created_at)
        .or_else(|_| {
            // Kick sometimes sends timestamps without timezone offset
            chrono::NaiveDateTime::parse_from_str(&msg.created_at, "%Y-%m-%dT%H:%M:%S")
                .map(|naive| naive.and_utc().fixed_offset())
        })
        .map_err(ParseError::Timestamp)?
        .timestamp_millis();
    let arrival_time = chrono::Utc::now().timestamp_millis();

    let identity = msg.sender.identity.unwrap_or(KickIdentity {
        color: None,
        badges: Vec::new(),
    });

    let is_broadcaster = identity
        .badges
        .iter()
        .any(|b| b.badge_type == "broadcaster");
    let is_mod = is_broadcaster || identity.badges.iter().any(|b| b.badge_type == "moderator");
    let is_subscriber = identity.badges.iter().any(|b| b.badge_type == "subscriber");

    let badges: Vec<Badge> = identity
        .badges
        .into_iter()
        .map(|b| Badge {
            set_id: format!("kick/{}", b.badge_type),
            id: String::from("1"),
        })
        .collect();

    Ok(Some(UnifiedMessage {
        id: msg.id,
        platform: Platform::Kick,
        timestamp: platform_ts,
        arrival_time,
        effective_ts: 0,
        arrival_seq: 0,
        username: msg.sender.username.clone(),
        display_name: msg.sender.username,
        platform_user_id: msg.sender.id.to_string(),
        message_text: msg.content,
        badges,
        is_mod,
        is_subscriber,
        is_broadcaster,
        color: identity.color,
        reply_to: None,
        emote_spans: Vec::new(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWER_MSG: &[u8] = br##"{
        "metadata": {
            "message_id": "meta-1",
            "message_type": "notification",
            "message_timestamp": "2023-11-06T18:11:47.492Z"
        },
        "payload": {
            "subscription": {
                "id": "sub-1",
                "status": "enabled",
                "type": "channel.chat.message",
                "version": "1",
                "cost": 0
            },
            "event": {
                "broadcaster_user_id": "1971641",
                "broadcaster_user_login": "streamer",
                "broadcaster_user_name": "streamer",
                "chatter_user_id": "4145994",
                "chatter_user_login": "viewer32",
                "chatter_user_name": "viewer32",
                "message_id": "cc106a89-1814-919d-454c-f4f2f970aae7",
                "message": {
                    "text": "Hi chat",
                    "fragments": []
                },
                "color": "#00FF7F",
                "badges": [],
                "message_type": "text"
            }
        }
    }"##;

    const MOD_MSG: &[u8] = br##"{
        "metadata": {
            "message_id": "meta-2",
            "message_type": "notification",
            "message_timestamp": "2023-11-06T18:11:48.100Z"
        },
        "payload": {
            "subscription": { "type": "channel.chat.message" },
            "event": {
                "broadcaster_user_id": "1",
                "chatter_user_id": "99",
                "chatter_user_login": "the_mod",
                "chatter_user_name": "TheMod",
                "message_id": "m-1",
                "message": { "text": "!ban spammer" },
                "color": "#FF0000",
                "badges": [
                    { "set_id": "moderator", "id": "1", "info": "" },
                    { "set_id": "subscriber", "id": "6", "info": "6" }
                ]
            }
        }
    }"##;

    const BROADCASTER_MSG: &[u8] = br##"{
        "metadata": {
            "message_id": "meta-3",
            "message_type": "notification",
            "message_timestamp": "2023-11-06T18:12:00.000Z"
        },
        "payload": {
            "subscription": { "type": "channel.chat.message" },
            "event": {
                "broadcaster_user_id": "1",
                "chatter_user_id": "1",
                "chatter_user_login": "streamer",
                "chatter_user_name": "Streamer",
                "message_id": "b-1",
                "message": { "text": "welcome everyone" },
                "badges": [{ "set_id": "broadcaster", "id": "1", "info": "" }]
            }
        }
    }"##;

    const REPLY_MSG: &[u8] = br##"{
        "metadata": {
            "message_id": "meta-4",
            "message_type": "notification",
            "message_timestamp": "2023-11-06T18:12:05.250Z"
        },
        "payload": {
            "subscription": { "type": "channel.chat.message" },
            "event": {
                "chatter_user_id": "42",
                "chatter_user_login": "replier",
                "chatter_user_name": "Replier",
                "message_id": "r-1",
                "message": { "text": "lol" },
                "reply": {
                    "parent_message_id": "parent-abc",
                    "parent_user_id": "1",
                    "parent_user_login": "streamer"
                }
            }
        }
    }"##;

    const KEEPALIVE: &[u8] = br##"{
        "metadata": {
            "message_id": "ka-1",
            "message_type": "session_keepalive",
            "message_timestamp": "2023-11-06T18:11:49.000Z"
        },
        "payload": {}
    }"##;

    const OTHER_NOTIFICATION: &[u8] = br##"{
        "metadata": {
            "message_id": "on-1",
            "message_type": "notification",
            "message_timestamp": "2023-11-06T18:11:50.000Z"
        },
        "payload": {
            "subscription": { "type": "channel.follow" },
            "event": {}
        }
    }"##;

    #[test]
    fn parses_viewer_message() {
        let msg = parse_twitch_envelope(VIEWER_MSG).unwrap().unwrap();
        assert_eq!(msg.id, "cc106a89-1814-919d-454c-f4f2f970aae7");
        assert_eq!(msg.username, "viewer32");
        assert_eq!(msg.display_name, "viewer32");
        assert_eq!(msg.platform_user_id, "4145994");
        assert_eq!(msg.message_text, "Hi chat");
        assert_eq!(msg.color.as_deref(), Some("#00FF7F"));
        assert!(matches!(msg.platform, Platform::Twitch));
        assert!(!msg.is_mod);
        assert!(!msg.is_subscriber);
        assert!(!msg.is_broadcaster);
        assert!(msg.reply_to.is_none());
        assert!(msg.timestamp > 0);
        assert!(msg.arrival_time >= msg.timestamp || msg.arrival_time > 0);
    }

    #[test]
    fn parses_moderator_flags() {
        let msg = parse_twitch_envelope(MOD_MSG).unwrap().unwrap();
        assert!(msg.is_mod);
        assert!(msg.is_subscriber);
        assert!(!msg.is_broadcaster);
        assert_eq!(msg.badges.len(), 2);
    }

    #[test]
    fn broadcaster_implies_mod() {
        let msg = parse_twitch_envelope(BROADCASTER_MSG).unwrap().unwrap();
        assert!(msg.is_broadcaster);
        assert!(msg.is_mod);
    }

    #[test]
    fn parses_reply_parent_id() {
        let msg = parse_twitch_envelope(REPLY_MSG).unwrap().unwrap();
        assert_eq!(msg.reply_to.as_deref(), Some("parent-abc"));
    }

    #[test]
    fn keepalive_returns_none() {
        let result = parse_twitch_envelope(KEEPALIVE).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn other_notification_types_return_none() {
        let result = parse_twitch_envelope(OTHER_NOTIFICATION).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn malformed_json_returns_err() {
        let err = parse_twitch_envelope(b"not json").unwrap_err();
        assert!(matches!(err, ParseError::Json(_)));
    }

    #[test]
    fn bad_timestamp_returns_err() {
        let bytes: &[u8] = br##"{
            "metadata": {
                "message_id": "m",
                "message_type": "notification",
                "message_timestamp": "not a date"
            },
            "payload": {
                "subscription": { "type": "channel.chat.message" },
                "event": {
                    "chatter_user_id": "1",
                    "chatter_user_login": "u",
                    "chatter_user_name": "U",
                    "message_id": "m",
                    "message": { "text": "x" }
                }
            }
        }"##;
        let err = parse_twitch_envelope(bytes).unwrap_err();
        assert!(matches!(err, ParseError::Timestamp(_)));
    }

    #[test]
    fn parse_error_display_and_source() {
        let err = parse_twitch_envelope(b"}{").unwrap_err();
        let _ = err.to_string();
        assert!(std::error::Error::source(&err).is_some());
    }

    // --- YouTube parser tests ---

    const YT_TEXT_MSG: &[u8] = br##"{
        "id": "yt-msg-1",
        "snippet": {
            "type": "TEXT_MESSAGE_EVENT",
            "live_chat_id": "chat123",
            "author_channel_id": "UC_abc",
            "published_at": "2024-06-15T12:30:00Z",
            "has_display_content": true,
            "display_message": "hello youtube",
            "text_message_details": {
                "message_text": "hello youtube"
            }
        },
        "author_details": {
            "channel_id": "UC_abc",
            "display_name": "TestViewer",
            "is_verified": false,
            "is_chat_owner": false,
            "is_chat_sponsor": false,
            "is_chat_moderator": false
        }
    }"##;

    const YT_OWNER_MSG: &[u8] = br##"{
        "id": "yt-msg-2",
        "snippet": {
            "type": "TEXT_MESSAGE_EVENT",
            "published_at": "2024-06-15T12:31:00Z",
            "text_message_details": { "message_text": "welcome all" }
        },
        "author_details": {
            "channel_id": "UC_owner",
            "display_name": "Streamer",
            "is_chat_owner": true,
            "is_chat_moderator": false,
            "is_chat_sponsor": false
        }
    }"##;

    const YT_MOD_MSG: &[u8] = br##"{
        "id": "yt-msg-3",
        "snippet": {
            "type": "TEXT_MESSAGE_EVENT",
            "published_at": "2024-06-15T12:32:00Z",
            "text_message_details": { "message_text": "calm down chat" }
        },
        "author_details": {
            "channel_id": "UC_mod",
            "display_name": "ModUser",
            "is_chat_owner": false,
            "is_chat_moderator": true,
            "is_chat_sponsor": true
        }
    }"##;

    const YT_SUPER_CHAT: &[u8] = br##"{
        "id": "yt-sc-1",
        "snippet": {
            "type": "SUPER_CHAT_EVENT",
            "published_at": "2024-06-15T12:33:00Z",
            "display_message": "$5.00",
            "super_chat_details": { "amount_micros": 5000000, "currency": "USD" }
        },
        "author_details": {
            "channel_id": "UC_donor",
            "display_name": "BigDonor"
        }
    }"##;

    #[test]
    fn parses_youtube_text_message() {
        let msg = parse_youtube_message(YT_TEXT_MSG).unwrap().unwrap();
        assert_eq!(msg.id, "yt-msg-1");
        assert!(matches!(msg.platform, Platform::YouTube));
        assert_eq!(msg.display_name, "TestViewer");
        assert_eq!(msg.platform_user_id, "UC_abc");
        assert_eq!(msg.message_text, "hello youtube");
        assert!(!msg.is_mod);
        assert!(!msg.is_subscriber);
        assert!(!msg.is_broadcaster);
        assert!(msg.color.is_none());
        assert!(msg.badges.is_empty());
        assert!(msg.timestamp > 0);
    }

    #[test]
    fn youtube_owner_implies_mod() {
        let msg = parse_youtube_message(YT_OWNER_MSG).unwrap().unwrap();
        assert!(msg.is_broadcaster);
        assert!(msg.is_mod);
        assert_eq!(msg.display_name, "Streamer");
        assert_eq!(msg.badges.len(), 1);
        assert_eq!(msg.badges[0].set_id, "youtube/owner");
        assert_eq!(msg.badges[0].id, "1");
        assert_eq!(msg.color.as_deref(), Some("#ffd600"));
    }

    #[test]
    fn youtube_moderator_flags() {
        let msg = parse_youtube_message(YT_MOD_MSG).unwrap().unwrap();
        assert!(msg.is_mod);
        assert!(msg.is_subscriber); // is_chat_sponsor maps to subscriber
        assert!(!msg.is_broadcaster);
        assert_eq!(msg.badges.len(), 2);
        assert_eq!(msg.badges[0].set_id, "youtube/moderator");
        assert_eq!(msg.badges[1].set_id, "youtube/member");
        assert_eq!(msg.color.as_deref(), Some("#5e84f1"));
    }

    #[test]
    fn youtube_non_text_returns_none() {
        let result = parse_youtube_message(YT_SUPER_CHAT).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn youtube_malformed_json_returns_err() {
        let err = parse_youtube_message(b"not json").unwrap_err();
        assert!(matches!(err, ParseError::Json(_)));
    }

    #[test]
    fn youtube_missing_snippet_returns_none() {
        let msg = br#"{"id":"x","author_details":{"channel_id":"c","display_name":"d"}}"#;
        assert!(parse_youtube_message(msg).unwrap().is_none());
    }

    #[test]
    fn youtube_missing_text_returns_none() {
        let msg = br##"{
            "id": "yt-empty",
            "snippet": {
                "type": "TEXT_MESSAGE_EVENT",
                "published_at": "2024-06-15T12:30:00Z"
            },
            "author_details": { "channel_id": "c", "display_name": "d" }
        }"##;
        assert!(parse_youtube_message(msg).unwrap().is_none());
    }

    #[test]
    fn youtube_empty_text_returns_none() {
        let msg = br##"{
            "id": "yt-empty",
            "snippet": {
                "type": "TEXT_MESSAGE_EVENT",
                "published_at": "2024-06-15T12:30:00Z",
                "text_message_details": { "message_text": "" }
            },
            "author_details": { "channel_id": "c", "display_name": "d" }
        }"##;
        assert!(parse_youtube_message(msg).unwrap().is_none());
    }

    #[test]
    fn youtube_missing_author_returns_none() {
        let msg = br##"{
            "id": "yt-1",
            "snippet": {
                "type": "TEXT_MESSAGE_EVENT",
                "published_at": "2024-06-15T12:30:00Z",
                "text_message_details": { "message_text": "hi" }
            }
        }"##;
        assert!(parse_youtube_message(msg).unwrap().is_none());
    }

    #[test]
    fn youtube_missing_id_returns_none() {
        let msg = br##"{
            "snippet": {
                "type": "TEXT_MESSAGE_EVENT",
                "published_at": "2024-06-15T12:30:00Z",
                "text_message_details": { "message_text": "hi" }
            },
            "author_details": { "channel_id": "c", "display_name": "d" }
        }"##;
        assert!(parse_youtube_message(msg).unwrap().is_none());
    }

    #[test]
    fn youtube_bad_timestamp_returns_err() {
        let msg = br##"{
            "id": "yt-1",
            "snippet": {
                "type": "TEXT_MESSAGE_EVENT",
                "published_at": "not a date",
                "text_message_details": { "message_text": "hi" }
            },
            "author_details": { "channel_id": "c", "display_name": "d" }
        }"##;
        let err = parse_youtube_message(msg).unwrap_err();
        assert!(matches!(err, ParseError::Timestamp(_)));
    }

    #[test]
    fn youtube_missing_timestamp_uses_now() {
        let msg = br##"{
            "id": "yt-1",
            "snippet": {
                "type": "TEXT_MESSAGE_EVENT",
                "text_message_details": { "message_text": "hi" }
            },
            "author_details": { "channel_id": "c", "display_name": "d" }
        }"##;
        let parsed = parse_youtube_message(msg).unwrap().unwrap();
        assert!(parsed.timestamp > 0);
    }

    // --- Kick parser tests ---

    const KICK_CHAT_EVENT: &[u8] = br##"{
        "event": "App\\Events\\ChatMessageEvent",
        "data": "{\"id\":\"msg-k1\",\"chatroom_id\":100,\"content\":\"hello kick\",\"type\":\"message\",\"created_at\":\"2025-06-01T12:00:00Z\",\"sender\":{\"id\":42,\"username\":\"viewer1\",\"slug\":\"viewer1\",\"identity\":{\"color\":\"#FF5733\",\"badges\":[]}}}",
        "channel": "chatrooms.100.v2"
    }"##;

    const KICK_MOD_MSG: &[u8] = br##"{
        "event": "App\\Events\\ChatMessageEvent",
        "data": "{\"id\":\"msg-k2\",\"chatroom_id\":100,\"content\":\"!ban spammer\",\"type\":\"message\",\"created_at\":\"2025-06-01T12:01:00Z\",\"sender\":{\"id\":99,\"username\":\"the_mod\",\"slug\":\"the_mod\",\"identity\":{\"color\":\"#FF0000\",\"badges\":[{\"type\":\"moderator\",\"text\":\"Moderator\"},{\"type\":\"subscriber\",\"text\":\"Subscriber\"}]}}}",
        "channel": "chatrooms.100.v2"
    }"##;

    const KICK_OTHER_EVENT: &[u8] = br##"{
        "event": "App\\Events\\UserBannedEvent",
        "data": "{}",
        "channel": "chatrooms.100.v2"
    }"##;

    #[test]
    fn parses_kick_chat_message() {
        let msg = parse_kick_event(KICK_CHAT_EVENT).unwrap().unwrap();
        assert_eq!(msg.id, "msg-k1");
        assert_eq!(msg.username, "viewer1");
        assert_eq!(msg.display_name, "viewer1");
        assert_eq!(msg.platform_user_id, "42");
        assert_eq!(msg.message_text, "hello kick");
        assert_eq!(msg.color.as_deref(), Some("#FF5733"));
        assert!(matches!(msg.platform, Platform::Kick));
        assert!(!msg.is_mod);
        assert!(!msg.is_subscriber);
        assert!(!msg.is_broadcaster);
        assert!(msg.timestamp > 0);
    }

    #[test]
    fn parses_kick_moderator_flags() {
        let msg = parse_kick_event(KICK_MOD_MSG).unwrap().unwrap();
        assert!(msg.is_mod);
        assert!(msg.is_subscriber);
        assert!(!msg.is_broadcaster);
        assert_eq!(msg.badges.len(), 2);
        assert_eq!(msg.badges[0].set_id, "kick/moderator");
        assert_eq!(msg.badges[0].id, "1");
        assert_eq!(msg.badges[1].set_id, "kick/subscriber");
        assert_eq!(msg.badges[1].id, "1");
    }

    #[test]
    fn kick_non_chat_event_returns_none() {
        let result = parse_kick_event(KICK_OTHER_EVENT).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn kick_malformed_json_returns_err() {
        let err = parse_kick_event(b"not json").unwrap_err();
        assert!(matches!(err, ParseError::Json(_)));
    }

    fn fake_msg(timestamp: i64, arrival_time: i64) -> UnifiedMessage {
        UnifiedMessage {
            id: String::new(),
            platform: Platform::Twitch,
            timestamp,
            arrival_time,
            effective_ts: 0,
            arrival_seq: 0,
            username: String::new(),
            display_name: String::new(),
            platform_user_id: String::new(),
            message_text: String::new(),
            badges: Vec::new(),
            is_mod: false,
            is_subscriber: false,
            is_broadcaster: false,
            color: None,
            reply_to: None,
            emote_spans: Vec::new(),
        }
    }

    #[test]
    fn snap_uses_platform_ts_when_within_window() {
        // Exactly at 0 delta → trust platform.
        assert_eq!(compute_effective_ts(1_000, 1_000), 1_000);
        // Inside window on either side.
        assert_eq!(compute_effective_ts(1_000, 1_400), 1_000);
        assert_eq!(compute_effective_ts(1_400, 1_000), 1_400);
    }

    #[test]
    fn snap_boundary_inclusive() {
        // Delta exactly at SNAP_WINDOW_MS still trusts platform.
        assert_eq!(compute_effective_ts(1_000, 1_000 + SNAP_WINDOW_MS), 1_000);
        assert_eq!(
            compute_effective_ts(1_000 + SNAP_WINDOW_MS, 1_000),
            1_000 + SNAP_WINDOW_MS
        );
    }

    #[test]
    fn snap_falls_back_to_arrival_when_outside_window() {
        // One ms past the window → arrival wins.
        assert_eq!(
            compute_effective_ts(1_000, 1_000 + SNAP_WINDOW_MS + 1),
            1_000 + SNAP_WINDOW_MS + 1
        );
        // Negative delta past the window also flips to arrival.
        assert_eq!(compute_effective_ts(10_000, 1_000), 1_000);
    }

    #[test]
    fn assign_arrival_seqs_assigns_in_order_and_advances_counter() {
        let mut batch = vec![fake_msg(0, 0), fake_msg(0, 0), fake_msg(0, 0)];
        let mut counter: u64 = 100;
        assign_arrival_seqs(&mut batch, &mut counter);
        assert_eq!(batch[0].arrival_seq, 100);
        assert_eq!(batch[1].arrival_seq, 101);
        assert_eq!(batch[2].arrival_seq, 102);
        assert_eq!(counter, 103);
    }

    #[test]
    fn assign_arrival_seqs_continues_across_batches() {
        let mut counter: u64 = 0;
        let mut first = vec![fake_msg(0, 0), fake_msg(0, 0)];
        assign_arrival_seqs(&mut first, &mut counter);
        let mut second = vec![fake_msg(0, 0)];
        assign_arrival_seqs(&mut second, &mut counter);
        assert_eq!(first[0].arrival_seq, 0);
        assert_eq!(first[1].arrival_seq, 1);
        assert_eq!(second[0].arrival_seq, 2);
        assert_eq!(counter, 3);
    }

    #[test]
    fn assign_arrival_seqs_empty_batch_is_noop() {
        let mut batch: Vec<UnifiedMessage> = Vec::new();
        let mut counter: u64 = 7;
        assign_arrival_seqs(&mut batch, &mut counter);
        assert_eq!(counter, 7);
    }
}
