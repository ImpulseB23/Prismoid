package control

// Bootstrap is the first message the Rust host writes to the sidecar's stdin
// at startup. It hands over the inherited shared memory section so the sidecar
// can attach without having to know a name or open a kernel object by lookup.
type Bootstrap struct {
	ShmHandle uintptr `json:"shm_handle"`
	ShmSize   int     `json:"shm_size"`
}

// Command is a control-plane message from the Rust host received over stdin
// after the Bootstrap message. Each command targets a single platform client.
type Command struct {
	Cmd           string `json:"cmd"`
	Channel       string `json:"channel,omitempty"`
	Token         string `json:"token,omitempty"`
	ClientID      string `json:"client_id,omitempty"`
	BroadcasterID string `json:"broadcaster_id,omitempty"`
	UserID        string `json:"user_id,omitempty"`
}

// Message is a notification the sidecar writes to stdout for the Rust host.
type Message struct {
	Type    string `json:"type"`
	Payload any    `json:"payload,omitempty"`
}
