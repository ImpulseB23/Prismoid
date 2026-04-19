package kick

import "encoding/json"

// PusherEvent is the top-level frame for all Pusher WebSocket messages.
// Protocol events (connection_established, pong, error) and channel events
// (ChatMessageEvent) share this shape. Data is always a JSON-encoded string.
type PusherEvent struct {
	Event   string `json:"event"`
	Data    string `json:"data"`
	Channel string `json:"channel,omitempty"`
}

// ConnectionData is the payload inside a pusher:connection_established event.
type ConnectionData struct {
	SocketID        string `json:"socket_id"`
	ActivityTimeout int    `json:"activity_timeout"`
}

// ChatMessage is the inner payload of a ChatMessageEvent on a chatrooms.*.v2
// Pusher channel. Field names follow the Pusher v2 format observed in the
// wild; the official webhook format uses slightly different names (message_id
// vs id, user_id vs id on sender, username_color vs color on identity).
type ChatMessage struct {
	ID         string          `json:"id"`
	ChatroomID int             `json:"chatroom_id,omitempty"`
	Content    string          `json:"content"`
	Type       string          `json:"type,omitempty"`
	CreatedAt  string          `json:"created_at"`
	Sender     Sender          `json:"sender"`
	Emotes     json.RawMessage `json:"emotes,omitempty"`
}

type Sender struct {
	ID       int      `json:"id"`
	Username string   `json:"username"`
	Slug     string   `json:"slug,omitempty"`
	Identity Identity `json:"identity"`
}

type Identity struct {
	Color  string  `json:"color"`
	Badges []Badge `json:"badges"`
}

type Badge struct {
	Type  string `json:"type"`
	Text  string `json:"text"`
	Count int    `json:"count,omitempty"`
}
