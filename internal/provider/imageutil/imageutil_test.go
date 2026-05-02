package imageutil

import (
	"encoding/base64"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestResolveFile(t *testing.T) {
	dir := t.TempDir()
	png := []byte{0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A}
	path := filepath.Join(dir, "test.png")
	if err := os.WriteFile(path, png, 0644); err != nil {
		t.Fatal(err)
	}

	mime, b64, err := Resolve("file://" + path)
	if err != nil {
		t.Fatalf("resolve file: %v", err)
	}
	if mime != "image/png" {
		t.Errorf("mime = %q, want image/png", mime)
	}
	want := base64.StdEncoding.EncodeToString(png)
	if b64 != want {
		t.Errorf("b64 mismatch")
	}
}

func TestResolveBarePath(t *testing.T) {
	dir := t.TempDir()
	jpeg := []byte{0xFF, 0xD8, 0xFF, 0xE0}
	path := filepath.Join(dir, "test.jpg")
	if err := os.WriteFile(path, jpeg, 0644); err != nil {
		t.Fatal(err)
	}

	mime, _, err := Resolve(path)
	if err != nil {
		t.Fatalf("resolve bare path: %v", err)
	}
	if mime != "image/jpeg" {
		t.Errorf("mime = %q, want image/jpeg", mime)
	}
}

func TestResolveDataURI(t *testing.T) {
	data := "data:image/png;base64,aGVsbG8="
	mime, b64, err := Resolve(data)
	if err != nil {
		t.Fatalf("resolve data uri: %v", err)
	}
	if mime != "image/png" {
		t.Errorf("mime = %q, want image/png", mime)
	}
	if b64 != "aGVsbG8=" {
		t.Errorf("b64 = %q, want aGVsbG8=", b64)
	}
}

func TestResolveDataURINoMime(t *testing.T) {
	data := "data:;base64,aGVsbG8="
	mime, _, err := Resolve(data)
	if err != nil {
		t.Fatalf("resolve data uri: %v", err)
	}
	// When mime is empty, it falls through to application/octet-stream
	if mime != "application/octet-stream" {
		t.Errorf("mime = %q, want application/octet-stream", mime)
	}
}

func TestDetectMime(t *testing.T) {
	tests := []struct {
		name string
		data []byte
		path string
		want string
	}{
		{"png", []byte{0x89, 0x50, 0x4E, 0x47}, "x.png", "image/png"},
		{"jpeg", []byte{0xFF, 0xD8, 0xFF}, "x.jpg", "image/jpeg"},
		{"gif", []byte{'G', 'I', 'F', '8', '9', 'a'}, "x.gif", "image/gif"},
		{"webp", []byte{'R', 'I', 'F', 'F', 0, 0, 0, 0, 'W', 'E', 'B', 'P'}, "x.webp", "image/webp"},
		{"unknown", []byte{0, 1, 2}, "x.bin", "image/png"},
		{"by-ext-jpeg", []byte{0, 1, 2}, "x.jpeg", "image/jpeg"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := detectMime(tt.path, tt.data)
			if got != tt.want {
				t.Errorf("detectMime(%q) = %q, want %q", tt.path, got, tt.want)
			}
		})
	}
}

func TestResolveMissingFile(t *testing.T) {
	_, _, err := Resolve("file:///nonexistent/path.png")
	if err == nil {
		t.Fatal("expected error for missing file")
	}
	if !strings.Contains(err.Error(), "read image file") {
		t.Errorf("error = %q, want 'read image file'", err.Error())
	}
}
