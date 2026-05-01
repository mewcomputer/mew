package session

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"

	"mew/internal/message"
)

// Writer persists messages as JSONL.
type Writer struct {
	file *os.File
	enc  *json.Encoder
}

// Open creates or appends to a session file.
func Open(sessionID string) (*Writer, error) {
	dir, err := os.UserConfigDir()
	if err != nil {
		return nil, err
	}
	sdir := filepath.Join(dir, "mew", "sessions")
	if err := os.MkdirAll(sdir, 0750); err != nil {
		return nil, err
	}
	path := filepath.Join(sdir, sessionID+".jsonl")
	f, err := os.OpenFile(path, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0640)
	if err != nil {
		return nil, err
	}
	return &Writer{file: f, enc: json.NewEncoder(f)}, nil
}

// WriteMessage appends a single message.
func (w *Writer) WriteMessage(msg message.Message) error {
	if err := w.enc.Encode(msg); err != nil {
		return fmt.Errorf("session write: %w", err)
	}
	return nil
}

// Close flushes and closes the underlying file.
func (w *Writer) Close() error {
	return w.file.Close()
}
