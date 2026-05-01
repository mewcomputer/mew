package message

import (
	"encoding/json"
	"testing"

	"github.com/oklog/ulid/v2"
)

func mkULID() string {
	return ulid.MustNew(ulid.Now(), nil).String()
}

func TestPartsRoundTrip(t *testing.T) {
	tests := []struct {
		name string
		parts Parts
	}{
		{
			name: "text-only",
			parts: Parts{
				&TextPart{
					partBase: partBase{ID: mkULID(), MessageID: mkULID(), SessionID: mkULID()},
					Type:     "text",
					Text:     "hello world",
				},
			},
		},
		{
			name: "with-reasoning",
			parts: Parts{
				&ReasoningPart{
					partBase:  partBase{ID: mkULID(), MessageID: mkULID(), SessionID: mkULID()},
					Type:      "reasoning",
					Text:      "let me think...",
					Signature: "sig123",
				},
				&TextPart{
					partBase: partBase{ID: mkULID(), MessageID: mkULID(), SessionID: mkULID()},
					Type:     "text",
					Text:     "the answer is 42",
				},
			},
		},
		{
			name: "tool-call-pending",
			parts: Parts{
				&ToolCallPart{
					partBase: partBase{ID: mkULID(), MessageID: mkULID(), SessionID: mkULID()},
					Type:     "tool_call",
					ToolName: "echo",
					CallID:   "call_1",
					State: ToolState{
						Status: ToolPending,
						Input:  map[string]any{"input": "hi"},
						Time:   ToolTime{Start: 1234567890},
					},
				},
			},
		},
		{
			name: "tool-call-running",
			parts: Parts{
				&ToolCallPart{
					partBase: partBase{ID: mkULID(), MessageID: mkULID(), SessionID: mkULID()},
					Type:     "tool_call",
					ToolName: "bash",
					CallID:   "call_2",
					State: ToolState{
						Status: ToolRunning,
						Input:  map[string]any{"command": "ls"},
						Time:   ToolTime{Start: 1234567890, End: 1234567900},
					},
				},
			},
		},
		{
			name: "tool-call-completed",
			parts: Parts{
				&ToolCallPart{
					partBase: partBase{ID: mkULID(), MessageID: mkULID(), SessionID: mkULID()},
					Type:     "tool_call",
					ToolName: "echo",
					CallID:   "call_3",
					State: ToolState{
						Status: ToolCompleted,
						Input:  map[string]any{"input": "hello"},
						Output: "hello",
						Time:   ToolTime{Start: 100, End: 200},
					},
				},
				&ToolResultPart{
					partBase: partBase{ID: mkULID(), MessageID: mkULID(), SessionID: mkULID()},
					Type:     "tool_result",
					CallID:   "call_3",
				},
			},
		},
		{
			name: "tool-call-error",
			parts: Parts{
				&ToolCallPart{
					partBase: partBase{ID: mkULID(), MessageID: mkULID(), SessionID: mkULID()},
					Type:     "tool_call",
					ToolName: "bash",
					CallID:   "call_4",
					State: ToolState{
						Status: ToolError,
						Input:  map[string]any{"command": "rm -rf /"},
						Error:  "permission denied",
						Time:   ToolTime{Start: 300, End: 301},
					},
				},
			},
		},
		{
			name: "multi-part-mixed",
			parts: Parts{
				&TextPart{
					partBase: partBase{ID: mkULID(), MessageID: mkULID(), SessionID: mkULID()},
					Type:     "text",
					Text:     "first",
				},
				&ReasoningPart{
					partBase: partBase{ID: mkULID(), MessageID: mkULID(), SessionID: mkULID()},
					Type:      "reasoning",
					Text:      "thinking...",
				},
				&ToolCallPart{
					partBase: partBase{ID: mkULID(), MessageID: mkULID(), SessionID: mkULID()},
					Type:     "tool_call",
					ToolName: "read",
					CallID:   "call_5",
					State: ToolState{
						Status: ToolPending,
						Input:  map[string]any{"path": "/tmp/foo"},
						Time:   ToolTime{Start: 400},
					},
				},
				&FilePart{
					partBase: partBase{ID: mkULID(), MessageID: mkULID(), SessionID: mkULID()},
					Type:     "file",
					Mime:     "text/plain",
					Filename: "foo.txt",
					URL:      "file:///tmp/foo",
				},
				&CompactionPart{
					partBase: partBase{ID: mkULID(), MessageID: mkULID(), SessionID: mkULID()},
					Type:     "compaction",
					Auto:     true,
					Overflow: false,
					TailStartID: mkULID(),
				},
			},
		},
		{
			name: "message-round-trip",
			parts: Parts{
				&TextPart{
					partBase:  partBase{ID: mkULID(), MessageID: mkULID(), SessionID: mkULID()},
					Type:      "text",
					Text:      "hello",
					Synthetic: true,
				},
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			// Build a message to exercise message-level marshaling too.
			msg := Message{
				ID:        mkULID(),
				SessionID: mkULID(),
				Role:      RoleUser,
				Parts:     tt.parts,
				Time:      Time{Created: 1234567890000},
			}
			if tt.name == "message-round-trip" {
				msg.Assistant = &AssistantMeta{
					ProviderID: "opencode-zen",
					ModelID:    "glm-4",
					Cost:       0.001,
					Tokens:     Tokens{Input: 10, Output: 5, Reasoning: 2, CacheRead: 0, CacheWrite: 0},
					Finish:     "stop",
					Error:      nil,
				}
			}

			b, err := json.Marshal(msg)
			if err != nil {
				t.Fatalf("marshal: %v", err)
			}

			var got Message
			if err := json.Unmarshal(b, &got); err != nil {
				t.Fatalf("unmarshal: %v", err)
			}

			// Deep equality via re-marshal to avoid unexported fields.
			b2, _ := json.Marshal(got)
			if string(b) != string(b2) {
				t.Fatalf("round-trip mismatch:\n%s\nvs\n%s", b, b2)
			}
		})
	}
}

func TestPartsUnmarshalUnknownType(t *testing.T) {
	data := []byte(`[{"type":"unknown","id":"01ABCDEF1234567890ABCDEF"}]`)
	var parts Parts
	if err := json.Unmarshal(data, &parts); err == nil {
		t.Fatal("expected error for unknown part type")
	}
}

func TestPartsUnmarshalMissingType(t *testing.T) {
	data := []byte(`[{"id":"01ABCDEF1234567890ABCDEF"}]`)
	var parts Parts
	if err := json.Unmarshal(data, &parts); err == nil {
		t.Fatal("expected error for missing part type")
	}
}
