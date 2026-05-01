package anthropic

import (
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	"mew/internal/message"
	"mew/internal/provider"
)

func loadFixture(t *testing.T, name string) []byte {
	t.Helper()
	path := filepath.Join("testdata", name)
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read fixture %s: %v", path, err)
	}
	return data
}

func TestAdapterTextOnly(t *testing.T) {
	fixture := loadFixture(t, "text-only.sse")
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		w.WriteHeader(http.StatusOK)
		w.Write(fixture)
	}))
	defer srv.Close()

	a := New("test", srv.URL, "claude-3", "fake-key")
	ctx := t.Context()
	evCh, err := a.Stream(ctx, provider.Request{
		Messages: nil,
		Tools:    nil,
		System:   "",
	})
	if err != nil {
		t.Fatalf("stream start: %v", err)
	}

	var parts []provider.Event
	for ev := range evCh {
		parts = append(parts, ev)
	}

	if len(parts) == 0 {
		t.Fatal("expected events, got none")
	}

	last := parts[len(parts)-1]
	me, ok := last.(provider.EventMessageEnd)
	if !ok {
		t.Fatalf("expected EventMessageEnd, got %T", last)
	}
	if me.Finish != "stop" {
		t.Fatalf("expected finish=stop, got %q", me.Finish)
	}
}

func TestAdapterToolCall(t *testing.T) {
	fixture := loadFixture(t, "tool-call.sse")
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		w.WriteHeader(http.StatusOK)
		w.Write(fixture)
	}))
	defer srv.Close()

	a := New("test", srv.URL, "claude-3", "fake-key")
	ctx := t.Context()
	evCh, err := a.Stream(ctx, provider.Request{
		Messages: nil,
		Tools:    nil,
		System:   "",
	})
	if err != nil {
		t.Fatalf("stream start: %v", err)
	}

	var parts []provider.Event
	for ev := range evCh {
		parts = append(parts, ev)
	}

	if len(parts) == 0 {
		t.Fatal("expected events, got none")
	}

	last := parts[len(parts)-1]
	me, ok := last.(provider.EventMessageEnd)
	if !ok {
		t.Fatalf("expected EventMessageEnd, got %T", last)
	}
	if me.Finish != "tool_use" {
		t.Fatalf("expected finish=tool_use, got %q", me.Finish)
	}

	var found bool
	for _, ev := range parts {
		if ps, ok := ev.(provider.EventPartStart); ok {
			if tc, ok := ps.Part.(*message.ToolCallPart); ok {
				found = true
				if tc.ToolName != "echo" {
					t.Fatalf("expected tool name echo, got %q", tc.ToolName)
				}
				if tc.CallID != "tu_01" {
					t.Fatalf("expected call_id tu_01, got %q", tc.CallID)
				}
				if tc.State.Input == nil {
					t.Fatal("expected parsed input, got nil")
				}
				if tc.State.Input["input"] != "hi" {
					t.Fatalf("expected input=hi, got %v", tc.State.Input["input"])
				}
			}
		}
	}
	if !found {
		t.Fatal("expected a ToolCallPart start event")
	}
}
