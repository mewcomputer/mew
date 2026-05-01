package agent

import "mew/internal/message"

// Event is the discriminated union of events emitted by the agent core to the TUI.
type Event interface {
	agentEventType() string
}

// EventPartStart mirrors provider.EventPartStart.
type EventPartStart struct {
	Part message.Part
}

func (EventPartStart) agentEventType() string { return "part_start" }

// EventPartDelta mirrors provider.EventPartDelta.
type EventPartDelta struct {
	PartID string
	Field  string
	Delta  string
}

func (EventPartDelta) agentEventType() string { return "part_delta" }

// EventPartEnd mirrors provider.EventPartEnd.
type EventPartEnd struct {
	PartID string
}

func (EventPartEnd) agentEventType() string { return "part_end" }

// EventPartUpdated is emitted when a tool call's state changes.
type EventPartUpdated struct {
	PartID string
	Part   message.Part
}

func (EventPartUpdated) agentEventType() string { return "part_updated" }

// EventMessageEnd mirrors provider.EventMessageEnd.
type EventMessageEnd struct {
	Finish string
	Usage  message.Tokens
	Cost   float64
}

func (EventMessageEnd) agentEventType() string { return "message_end" }

// EventPermissionAsk requests user approval for a tool call.
type EventPermissionAsk struct {
	Call message.ToolCallPart
}

func (EventPermissionAsk) agentEventType() string { return "permission_ask" }

// EventError signals a terminal error.
type EventError struct {
	Err error
}

func (EventError) agentEventType() string { return "error" }
