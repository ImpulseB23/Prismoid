package control

type Command struct {
	Cmd     string `json:"cmd"`
	Channel string `json:"channel,omitempty"`
	Token   string `json:"token,omitempty"`
}

type Message struct {
	Type    string `json:"type"`
	Payload any    `json:"payload,omitempty"`
}
