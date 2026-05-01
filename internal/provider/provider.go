package provider

import (
	"context"
	"encoding/json"

	"mew/internal/message"
)

// Provider is the canonical interface for all LLM backends.
type Provider interface {
	Name() string
	Stream(ctx context.Context, req Request) (<-chan Event, error)
}

// Request is the canonical request shape sent to any provider.
type Request struct {
	Model    string
	Messages []message.Message
	Tools    []ToolDef
	System   string
}

// ToolDef describes a tool available to the model.
type ToolDef struct {
	Name        string
	Description string
	Schema      json.RawMessage
}

// Event is the discriminated union of events emitted by a provider adapter.
type Event interface {
	eventType() string
}

type EventPartStart struct {
	Part message.Part
}

func (EventPartStart) eventType() string { return "part_start" }

type EventPartDelta struct {
	PartID string
	Field  string
	Delta  string
}

func (EventPartDelta) eventType() string { return "part_delta" }

type EventPartEnd struct {
	PartID string
}

func (EventPartEnd) eventType() string { return "part_end" }

type EventMessageEnd struct {
	Finish string
	Usage  message.Tokens
	Cost   float64
}

func (EventMessageEnd) eventType() string { return "message_end" }

type EventError struct {
	Err error
}

func (EventError) eventType() string { return "error" }
