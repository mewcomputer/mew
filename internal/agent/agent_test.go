package agent

import (
	"context"
	"testing"
	"time"

	"github.com/oklog/ulid/v2"

	"mew/internal/hooks"
	"mew/internal/message"
	"mew/internal/provider"
	"mew/internal/provider/fake"
	"mew/internal/session"
	"mew/internal/tools"
)

func TestAgentTextOnly(t *testing.T) {
	partID := ulid.MustNew(ulid.Now(), nil).String()
	p := fake.New("fake", []provider.Event{
		fake.TextPartStart(partID),
		fake.TextDelta(partID, "Hello"),
		fake.TextDelta(partID, " world"),
		fake.PartEnd(partID),
		fake.MessageEnd("stop"),
	})

	sw, err := session.Open("test-session-" + ulid.MustNew(ulid.Now(), nil).String())
	if err != nil {
		t.Fatalf("open session: %v", err)
	}
	defer sw.Close()

	a := New(p, hooks.Nop(), sw, "test-session", []tools.Tool{tools.Echo{}})
	ctx := t.Context()
	evCh := make(chan Event)
	go func() {
		defer close(evCh)
		if err := a.Run(ctx, "say hello", evCh); err != nil {
			t.Errorf("run: %v", err)
		}
	}()

	var deltas []string
	for ev := range evCh {
		switch e := ev.(type) {
		case EventPartDelta:
			deltas = append(deltas, e.Delta)
		case EventMessageEnd:
			if e.Finish != "stop" {
				t.Fatalf("expected finish=stop, got %q", e.Finish)
			}
		case EventError:
			t.Fatalf("unexpected error: %v", e.Err)
		}
	}

	got := ""
	for _, d := range deltas {
		got += d
	}
	if got != "Hello world" {
		t.Fatalf("expected 'Hello world', got %q", got)
	}
}

func TestAgentToolExecution(t *testing.T) {
	textID := ulid.MustNew(ulid.Now(), nil).String()
	toolID := ulid.MustNew(ulid.Now(), nil).String()

	p := fake.New("fake", []provider.Event{
		fake.TextPartStart(textID),
		fake.TextDelta(textID, "Calling echo"),
		fake.PartEnd(textID),
		fake.ToolCallPartStart(toolID, "echo", "call_1", map[string]any{"input": "hi"}),
		fake.PartEnd(toolID),
		fake.MessageEnd("tool_use"),
	})

	sw, err := session.Open("test-session-" + ulid.MustNew(ulid.Now(), nil).String())
	if err != nil {
		t.Fatalf("open session: %v", err)
	}
	defer sw.Close()

	a := New(p, hooks.Nop(), sw, "test-session", []tools.Tool{tools.Echo{}})
	ctx := t.Context()
	evCh := make(chan Event)
	go func() {
		defer close(evCh)
		if err := a.Run(ctx, "call echo", evCh); err != nil {
			t.Errorf("run: %v", err)
		}
	}()

	var sawToolRunning, sawToolCompleted bool
	for ev := range evCh {
		switch e := ev.(type) {
		case EventPartUpdated:
			if tc, ok := e.Part.(*message.ToolCallPart); ok {
				switch tc.State.Status {
				case message.ToolRunning:
					sawToolRunning = true
				case message.ToolCompleted:
					sawToolCompleted = true
					if tc.State.Output != "echo: hi" {
						t.Fatalf("expected output 'echo: hi', got %q", tc.State.Output)
					}
				}
			}
		case EventError:
			t.Fatalf("unexpected error: %v", e.Err)
		}
	}

	if !sawToolRunning {
		t.Fatal("expected to see tool running event")
	}
	if !sawToolCompleted {
		t.Fatal("expected to see tool completed event")
	}
}

func TestAgentSessionPersistsMessages(t *testing.T) {
	partID := ulid.MustNew(ulid.Now(), nil).String()
	p := fake.New("fake", []provider.Event{
		fake.TextPartStart(partID),
		fake.TextDelta(partID, "done"),
		fake.PartEnd(partID),
		fake.MessageEnd("stop"),
	})

	sessionID := "test-session-" + ulid.MustNew(ulid.Now(), nil).String()
	sw, err := session.Open(sessionID)
	if err != nil {
		t.Fatalf("open session: %v", err)
	}
	defer sw.Close()

	a := New(p, hooks.Nop(), sw, sessionID, []tools.Tool{})
	ctx := t.Context()
	evCh := make(chan Event)
	go func() {
		defer close(evCh)
		if err := a.Run(ctx, "test", evCh); err != nil {
			t.Errorf("run: %v", err)
		}
	}()
	for range evCh {
	}

	// Session should have user message + assistant message.
	// We can't easily read back the jsonl here, but we verify the agent
	// has the correct message count in memory.
	if len(a.messages) != 2 {
		t.Fatalf("expected 2 messages in session, got %d", len(a.messages))
	}
	if a.messages[0].Role != message.RoleUser {
		t.Fatalf("expected first message to be user, got %s", a.messages[0].Role)
	}
	if a.messages[1].Role != message.RoleAssistant {
		t.Fatalf("expected second message to be assistant, got %s", a.messages[1].Role)
	}
}

func TestAgentAbort(t *testing.T) {
	partID := ulid.MustNew(ulid.Now(), nil).String()
	p := fake.New("fake", []provider.Event{
		fake.TextPartStart(partID),
		fake.TextDelta(partID, "Hello"),
	})

	sw, err := session.Open("test-session-" + ulid.MustNew(ulid.Now(), nil).String())
	if err != nil {
		t.Fatalf("open session: %v", err)
	}
	defer sw.Close()

	ctx, cancel := context.WithCancel(t.Context())
	a := New(p, hooks.Nop(), sw, "test-session", []tools.Tool{})
	evCh := make(chan Event)
	go func() {
		defer close(evCh)
		a.Run(ctx, "test", evCh)
	}()

	// Cancel after a short delay.
	time.Sleep(10 * time.Millisecond)
	cancel()

	var sawError bool
	for ev := range evCh {
		if _, ok := ev.(EventError); ok {
			sawError = true
		}
	}
	if !sawError {
		t.Fatal("expected error event on abort")
	}
}
