package tools

import (
	"context"
	"encoding/json"
)

// Tool is the interface for all tools available to the agent.
type Tool interface {
	Name() string
	Description() string
	Schema() json.RawMessage
	Execute(ctx context.Context, input map[string]any) (Result, error)
}

// Result is the output of a tool execution.
type Result struct {
	Output string
	Error  string
}
