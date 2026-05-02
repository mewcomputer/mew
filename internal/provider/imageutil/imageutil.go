// Package imageutil provides helpers for resolving image URLs to base64 data.
package imageutil

import (
	"encoding/base64"
	"fmt"
	"io"
	"net/http"
	"os"
	"strings"
)

// Resolve reads an image URL and returns the base64-encoded data and mime type.
// Supported URL schemes:
//   - file://  : read from disk, return base64
//   - data:    : parse data URI, return base64 payload
//   - https:// : fetch, return base64 (or pass through if allowed)
//   - http://  : fetch, return base64
func Resolve(url string) (mime string, b64 string, err error) {
	switch {
	case strings.HasPrefix(url, "file://"):
		return resolveFile(url)
	case strings.HasPrefix(url, "data:"):
		return resolveDataURI(url)
	case strings.HasPrefix(url, "http://") || strings.HasPrefix(url, "https://"):
		return resolveHTTP(url)
	default:
		// Treat bare path as file path.
		return resolveFile("file://" + url)
	}
}

func resolveFile(url string) (string, string, error) {
	path := strings.TrimPrefix(url, "file://")
	data, err := os.ReadFile(path)
	if err != nil {
		return "", "", fmt.Errorf("read image file: %w", err)
	}
	mime := detectMime(path, data)
	return mime, base64.StdEncoding.EncodeToString(data), nil
}

func resolveDataURI(url string) (string, string, error) {
	// data:[<mediatype>][;base64],<data>
	const prefix = "data:"
	rest := strings.TrimPrefix(url, prefix)
	commaIdx := strings.Index(rest, ",")
	if commaIdx < 0 {
		return "", "", fmt.Errorf("invalid data URI: no comma")
	}
	meta := rest[:commaIdx]
	data := rest[commaIdx+1:]

	mime := "application/octet-stream"
	if meta == "" {
		// no metadata
	} else if meta == "base64" {
		// no mime, just base64
	} else if idx := strings.Index(meta, ";"); idx >= 0 {
		if idx > 0 {
			mime = meta[:idx]
		}
	} else if meta != "base64" {
		mime = meta
	}

	if strings.Contains(meta, "base64") {
		return mime, data, nil
	}
	// URL-encoded data URI — decode to base64.
	decoded, err := io.ReadAll(strings.NewReader(data))
	if err != nil {
		return "", "", err
	}
	return mime, base64.StdEncoding.EncodeToString(decoded), nil
}

func resolveHTTP(url string) (string, string, error) {
	resp, err := http.Get(url)
	if err != nil {
		return "", "", fmt.Errorf("fetch image: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return "", "", fmt.Errorf("fetch image: status %d", resp.StatusCode)
	}
	data, err := io.ReadAll(resp.Body)
	if err != nil {
		return "", "", fmt.Errorf("read image body: %w", err)
	}
	mime := resp.Header.Get("Content-Type")
	if mime == "" {
		mime = detectMime(url, data)
	}
	return mime, base64.StdEncoding.EncodeToString(data), nil
}

func detectMime(path string, data []byte) string {
	// Sniff the first 512 bytes.
	if len(data) >= 8 {
		switch {
		case isPNG(data):
			return "image/png"
		case isJPEG(data):
			return "image/jpeg"
		case isGIF(data):
			return "image/gif"
		case isWebP(data):
			return "image/webp"
		}
	}
	// Fallback to extension.
	lower := strings.ToLower(path)
	switch {
	case strings.HasSuffix(lower, ".png"):
		return "image/png"
	case strings.HasSuffix(lower, ".jpg"), strings.HasSuffix(lower, ".jpeg"):
		return "image/jpeg"
	case strings.HasSuffix(lower, ".gif"):
		return "image/gif"
	case strings.HasSuffix(lower, ".webp"):
		return "image/webp"
	}
	return "image/png" // safest default
}

func isPNG(data []byte) bool {
	return len(data) >= 8 &&
		data[0] == 0x89 && data[1] == 0x50 && data[2] == 0x4E && data[3] == 0x47
}

func isJPEG(data []byte) bool {
	return len(data) >= 3 &&
		data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF
}

func isGIF(data []byte) bool {
	return len(data) >= 6 &&
		(data[0] == 'G' && data[1] == 'I' && data[2] == 'F')
}

func isWebP(data []byte) bool {
	return len(data) >= 12 &&
		data[0] == 'R' && data[1] == 'I' && data[2] == 'F' && data[3] == 'F' &&
		data[8] == 'W' && data[9] == 'E' && data[10] == 'B' && data[11] == 'P'
}
