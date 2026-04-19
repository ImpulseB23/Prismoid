package control

// Platform tag bytes prepended to each ring-buffer payload so the Rust host
// can dispatch to the correct parser without inspecting the JSON body.
const (
	TagTwitch  byte = 0x01
	TagKick    byte = 0x02
	TagYouTube byte = 0x03
)

// Bootstrap is the first message the Rust host writes to the sidecar's stdin
// at startup. It hands over the inherited shared memory section so the sidecar
// can attach without having to know a name or open a kernel object by lookup.
//
// ShmEventHandle is an auto-reset Windows Event created by the host alongside
// the mapping. The writer goroutine calls SetEvent on it after each successful
// ring write so the host can wake from WaitForSingleObject immediately instead
// of polling on a timer.
type Bootstrap struct {
	ShmHandle      uintptr `json:"shm_handle"`
	ShmEventHandle uintptr `json:"shm_event_handle"`
	ShmSize        int     `json:"shm_size"`
}

// Command is a control-plane message from the Rust host received over stdin
// after the Bootstrap message. Each command targets a single platform client.
//
// The Command is an intentionally flat struct with all possible fields as
// omitempty — not an envelope with a typed payload. When the handler count
// grows past ~8 distinct commands, split into envelope+args. Until then the
// flat shape is the simpler thing to serialize on the Rust side.
type Command struct {
	Cmd      string `json:"cmd"`
	Channel  string `json:"channel,omitempty"`
	Token    string `json:"token,omitempty"`
	ClientID string `json:"client_id,omitempty"`

	// BroadcasterID is the channel the command operates against. Used by
	// twitch_connect (subscribe to this channel's chat) and by mod actions
	// (the channel the mod action happens in).
	BroadcasterID string `json:"broadcaster_id,omitempty"`
	// UserID is the acting user's ID — the self ID for twitch_connect, the
	// moderator's ID for mod actions. Twitch Helix's moderation endpoints
	// require the moderator_id to match the token's authenticated user.
	UserID string `json:"user_id,omitempty"`

	// ChatroomID is the Kick chatroom numeric ID. Used by kick_connect to
	// subscribe to the Pusher channel for this chatroom.
	ChatroomID int `json:"chatroom_id,omitempty"`

	// Mod action fields. Only set by ban_user / unban_user / timeout_user /
	// delete_message commands.
	TargetUserID    string `json:"target_user_id,omitempty"`
	DurationSeconds int    `json:"duration_seconds,omitempty"`
	Reason          string `json:"reason,omitempty"`
	MessageID       string `json:"message_id,omitempty"`

	// Message body for send_chat_message. Capped at 500 bytes by Twitch's
	// Helix POST /chat/messages endpoint; the sidecar enforces the limit
	// before issuing the request.
	Message string `json:"message,omitempty"`

	// YouTube fields
	VideoID    string `json:"video_id,omitempty"`
	LiveChatID string `json:"live_chat_id,omitempty"`
	APIKey     string `json:"api_key,omitempty"`
}

// Message is a notification the sidecar writes to stdout for the Rust host.
type Message struct {
	Type    string `json:"type"`
	Payload any    `json:"payload,omitempty"`
}

// HeartbeatPayload is the body of a `{"type":"heartbeat", ...}` Message.
//
// The host uses this to detect a stalled sidecar even when the process is
// technically alive (hung WebSocket read, deadlocked goroutine): a gap in
// `Counter` signals a missed tick, and comparing `TSMs` to the host's own
// clock catches gross clock skew. Both fields are monotonic within a single
// sidecar run; they reset on respawn.
type HeartbeatPayload struct {
	TSMs    int64  `json:"ts_ms"`
	Counter uint64 `json:"counter"`
}
