package message


type Role string

const (
	RoleUser      Role = "user"
	RoleAssistant Role = "assistant"
)

type Message struct {
	ID        string         `json:"id"`
	SessionID string         `json:"sessionID"`
	Role      Role           `json:"role"`
	Parts     Parts          `json:"parts"`
	Time      Time           `json:"time"`
	Assistant *AssistantMeta `json:"assistant,omitempty"`
}

type Time struct {
	Created   int64 `json:"created"`
	Completed int64 `json:"completed,omitempty"`
}

type AssistantMeta struct {
	ProviderID string  `json:"providerID"`
	ModelID    string  `json:"modelID"`
	Cost       float64 `json:"cost"`
	Tokens     Tokens  `json:"tokens"`
	Finish     string  `json:"finish,omitempty"`
	Error      *Error  `json:"error,omitempty"`
}

type Tokens struct {
	Input      int `json:"input"`
	Output     int `json:"output"`
	Reasoning  int `json:"reasoning"`
	CacheRead  int `json:"cacheRead"`
	CacheWrite int `json:"cacheWrite"`
}

type Error struct {
	Kind    string `json:"kind"`
	Message string `json:"message"`
}
