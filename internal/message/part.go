package message

import (
	"encoding/json"
	"fmt"
)

// Part is the discriminated union of all message part types.
type Part interface {
	PartID() string
	partType() string
}

type partBase struct {
	ID        string `json:"id"`
	MessageID string `json:"messageID"`
	SessionID string `json:"sessionID"`
}

func (p partBase) PartID() string { return p.ID }

// --- concrete part types ---

type TextPart struct {
	partBase
	Type      string `json:"type"`
	Text      string `json:"text"`
	Synthetic bool   `json:"synthetic,omitempty"`
}

func (TextPart) partType() string { return "text" }

type ReasoningPart struct {
	partBase
	Type      string `json:"type"`
	Text      string `json:"text"`
	Signature string `json:"signature,omitempty"`
}

func (ReasoningPart) partType() string { return "reasoning" }

type FilePart struct {
	partBase
	Type     string `json:"type"`
	Mime     string `json:"mime"`
	Filename string `json:"filename,omitempty"`
	URL      string `json:"url"`
}

func (FilePart) partType() string { return "file" }

type ToolCallPart struct {
	partBase
	Type     string    `json:"type"`
	ToolName string    `json:"toolName"`
	CallID   string    `json:"callID"`
	State    ToolState `json:"state"`
}

func (ToolCallPart) partType() string { return "tool_call" }

type ToolResultPart struct {
	partBase
	Type   string `json:"type"`
	CallID string `json:"callID"`
}

func (ToolResultPart) partType() string { return "tool_result" }

type CompactionPart struct {
	partBase
	Type        string `json:"type"`
	Auto        bool   `json:"auto"`
	Overflow    bool   `json:"overflow,omitempty"`
	TailStartID string `json:"tailStartID,omitempty"`
}

func (CompactionPart) partType() string { return "compaction" }

// --- tool state ---

type ToolState struct {
	Status   ToolStatus     `json:"status"`
	Input    map[string]any `json:"input,omitempty"`
	Output   string         `json:"output,omitempty"`
	Error    string         `json:"error,omitempty"`
	Metadata map[string]any `json:"metadata,omitempty"`
	Time     ToolTime       `json:"time"`
}

type ToolStatus string

const (
	ToolPending   ToolStatus = "pending"
	ToolRunning   ToolStatus = "running"
	ToolCompleted ToolStatus = "completed"
	ToolError     ToolStatus = "error"
)

type ToolTime struct {
	Start int64 `json:"start"`
	End   int64 `json:"end,omitempty"`
}

// --- Parts slice with custom JSON ---

type Parts []Part

func (ps Parts) MarshalJSON() ([]byte, error) {
	// Parts is a []Part interface slice; let json.Marshal handle each element.
	// We need to marshal as the concrete types so the type field is included.
	tmp := make([]json.RawMessage, len(ps))
	for i, p := range ps {
		b, err := json.Marshal(p)
		if err != nil {
			return nil, err
		}
		tmp[i] = b
	}
	return json.Marshal(tmp)
}

func (ps *Parts) UnmarshalJSON(data []byte) error {
	var raw []json.RawMessage
	if err := json.Unmarshal(data, &raw); err != nil {
		return err
	}

	*ps = make(Parts, 0, len(raw))
	for i, r := range raw {
		var peek struct {
			Type string `json:"type"`
		}
		if err := json.Unmarshal(r, &peek); err != nil {
			return fmt.Errorf("part %d: cannot peek type: %w", i, err)
		}

		var p Part
		switch peek.Type {
		case "text":
			p = &TextPart{Type: "text"}
		case "reasoning":
			p = &ReasoningPart{Type: "reasoning"}
		case "file":
			p = &FilePart{Type: "file"}
		case "tool_call":
			p = &ToolCallPart{Type: "tool_call"}
		case "tool_result":
			p = &ToolResultPart{Type: "tool_result"}
		case "compaction":
			p = &CompactionPart{Type: "compaction"}
		default:
			return fmt.Errorf("part %d: unknown type %q", i, peek.Type)
		}

		if err := json.Unmarshal(r, p); err != nil {
			return fmt.Errorf("part %d (%s): %w", i, peek.Type, err)
		}
		*ps = append(*ps, p)
	}
	return nil
}

// --- constructors ---

func NewTextPart(id, msgID, sessionID, text string, synthetic bool) *TextPart {
	return &TextPart{
		partBase:  partBase{ID: id, MessageID: msgID, SessionID: sessionID},
		Type:      "text",
		Text:      text,
		Synthetic: synthetic,
	}
}

func NewReasoningPart(id, msgID, sessionID, text, signature string) *ReasoningPart {
	return &ReasoningPart{
		partBase:  partBase{ID: id, MessageID: msgID, SessionID: sessionID},
		Type:      "reasoning",
		Text:      text,
		Signature: signature,
	}
}

func NewFilePart(id, msgID, sessionID, mime, filename, url string) *FilePart {
	return &FilePart{
		partBase: partBase{ID: id, MessageID: msgID, SessionID: sessionID},
		Type:     "file",
		Mime:     mime,
		Filename: filename,
		URL:      url,
	}
}

func NewToolCallPart(id, msgID, sessionID, toolName, callID string, state ToolState) *ToolCallPart {
	return &ToolCallPart{
		partBase: partBase{ID: id, MessageID: msgID, SessionID: sessionID},
		Type:     "tool_call",
		ToolName: toolName,
		CallID:   callID,
		State:    state,
	}
}

func NewToolResultPart(id, msgID, sessionID, callID string) *ToolResultPart {
	return &ToolResultPart{
		partBase: partBase{ID: id, MessageID: msgID, SessionID: sessionID},
		Type:     "tool_result",
		CallID:   callID,
	}
}

func NewCompactionPart(id, msgID, sessionID string, auto, overflow bool, tailStartID string) *CompactionPart {
	return &CompactionPart{
		partBase:    partBase{ID: id, MessageID: msgID, SessionID: sessionID},
		Type:        "compaction",
		Auto:        auto,
		Overflow:    overflow,
		TailStartID: tailStartID,
	}
}

// --- mutation helpers ---

func (p *TextPart) WithText(delta string) *TextPart {
	return &TextPart{
		partBase:  p.partBase,
		Type:      p.Type,
		Text:      p.Text + delta,
		Synthetic: p.Synthetic,
	}
}

func (p *ReasoningPart) WithText(delta string) *ReasoningPart {
	return &ReasoningPart{
		partBase:  p.partBase,
		Type:      p.Type,
		Text:      p.Text + delta,
		Signature: p.Signature,
	}
}

func (p *ToolCallPart) WithState(state ToolState) *ToolCallPart {
	return &ToolCallPart{
		partBase: p.partBase,
		Type:     p.Type,
		ToolName: p.ToolName,
		CallID:   p.CallID,
		State:    state,
	}
}
