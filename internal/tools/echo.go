package tools

import (
	"context"
	"encoding/json"
	"fmt"
)

// Echo is a fake tool for M0 testing. It echoes back its input.
type Echo struct{}

func (Echo) Name() string        { return "echo" }
func (Echo) Description() string { return "Echoes back the provided input." }
func (Echo) Schema() json.RawMessage {
	return json.RawMessage(`{
		"type": "object",
		"properties": {
			"input": {
				"type": "string",
				"description": "The string to echo back."
			}
		},
		"required": ["input"]
	}`)
}

func (Echo) Execute(_ context.Context, input map[string]any) (Result, error) {
	v, ok := input["input"].(string)
	if !ok {
		return Result{Error: "missing or non-string input"}, nil
	}
	return Result{Output: fmt.Sprintf("echo: %s", v)}, nil
}
