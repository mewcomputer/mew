package session

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"

	"github.com/oklog/ulid/v2"

	"mew/internal/message"
)

func sessionDir(t *testing.T) string {
	t.Helper()
	dir, err := os.UserConfigDir()
	if err != nil {
		t.Fatalf("user config dir: %v", err)
	}
	return filepath.Join(dir, "mew", "sessions")
}

func TestWriterRoundTrip(t *testing.T) {
	sdir := sessionDir(t)
	if err := os.MkdirAll(sdir, 0750); err != nil {
		t.Fatalf("mkdir: %v", err)
	}

	sessionID := "test-session-" + ulid.MustNew(ulid.Now(), nil).String()
	w, err := Open(sessionID)
	if err != nil {
		t.Fatalf("open: %v", err)
	}

	msg := message.Message{
		ID:        ulid.MustNew(ulid.Now(), nil).String(),
		SessionID: sessionID,
		Role:      message.RoleUser,
		Parts: message.Parts{
			message.NewTextPart(ulid.MustNew(ulid.Now(), nil).String(), "", "", "hello", false),
		},
	}
	if err := w.WriteMessage(msg); err != nil {
		t.Fatalf("write: %v", err)
	}
	if err := w.Close(); err != nil {
		t.Fatalf("close: %v", err)
	}

	// Clean up after test.
	defer os.Remove(filepath.Join(sdir, sessionID+".jsonl"))

	data, err := os.ReadFile(filepath.Join(sdir, sessionID + ".jsonl"))
	if err != nil {
		t.Fatalf("read file: %v", err)
	}

	var got message.Message
	if err := json.Unmarshal(data, &got); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if got.Role != msg.Role {
		t.Fatalf("expected role %s, got %s", msg.Role, got.Role)
	}
	if len(got.Parts) != 1 {
		t.Fatalf("expected 1 part, got %d", len(got.Parts))
	}
	if tp, ok := got.Parts[0].(*message.TextPart); !ok || tp.Text != "hello" {
		t.Fatalf("expected text part with 'hello', got %v", got.Parts[0])
	}
}
