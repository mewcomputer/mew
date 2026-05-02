package catalog

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestParseValidCatalog(t *testing.T) {
	payload := map[string]any{
		"models": []map[string]any{
			{
				"id":             "z-ai/glm-4.6",
				"provider":       "z-ai",
				"context_window": 128000,
				"max_output":     4096,
				"tool_call":      true,
				"reasoning":      false,
				"vision":         true,
				"shape":          "anthropic",
				"pricing": map[string]any{
					"input":  0.5,
					"output": 1.5,
				},
			},
			{
				"id":             "opencode-zen/deepseek-v4-flash",
				"provider":       "opencode-zen",
				"context_window": 64000,
				"max_output":     8192,
				"tool_call":      true,
				"reasoning":      true,
				"vision":         false,
				"shape":          "openai",
				"pricing": map[string]any{
					"input":  0.1,
					"output": 0.3,
				},
			},
		},
	}
	data, _ := json.Marshal(payload)

	cat, err := parse(data)
	if err != nil {
		t.Fatalf("parse: %v", err)
	}

	m, ok := cat.Lookup("z-ai/glm-4.6")
	if !ok {
		t.Fatal("expected to find z-ai/glm-4.6")
	}
	if m.Provider != "z-ai" {
		t.Errorf("provider = %q, want z-ai", m.Provider)
	}
	if m.ContextWindow != 128000 {
		t.Errorf("context_window = %d, want 128000", m.ContextWindow)
	}
	if !m.Vision {
		t.Error("expected vision = true")
	}
	if m.Shape != "anthropic" {
		t.Errorf("shape = %q, want anthropic", m.Shape)
	}
	if m.Pricing.Input != 0.5 {
		t.Errorf("input price = %f, want 0.5", m.Pricing.Input)
	}

	_, ok = cat.Lookup("unknown/model")
	if ok {
		t.Error("expected not to find unknown/model")
	}
}

func TestShapeFor(t *testing.T) {
	cat, _ := parse([]byte(`{"models":[{"id":"a","shape":"anthropic"},{"id":"b","shape":"openai"}]}`))
	if got := cat.ShapeFor("a"); got != "anthropic" {
		t.Errorf("ShapeFor(a) = %q, want anthropic", got)
	}
	if got := cat.ShapeFor("b"); got != "openai" {
		t.Errorf("ShapeFor(b) = %q, want openai", got)
	}
	if got := cat.ShapeFor("unknown"); got != "openai" {
		t.Errorf("ShapeFor(unknown) = %q, want openai", got)
	}
}

func TestContextWindow(t *testing.T) {
	cat, _ := parse([]byte(`{"models":[{"id":"a","context_window":32000}]}`))
	if got := cat.ContextWindow("a"); got != 32000 {
		t.Errorf("ContextWindow(a) = %d, want 32000", got)
	}
	if got := cat.ContextWindow("unknown"); got != 128000 {
		t.Errorf("ContextWindow(unknown) = %d, want 128000", got)
	}
}

func TestBooleanFlags(t *testing.T) {
	cat, _ := parse([]byte(`{"models":[{"id":"m","vision":true,"tool_call":true,"reasoning":true}]}`))
	if !cat.SupportsVision("m") {
		t.Error("SupportsVision should be true")
	}
	if !cat.SupportsToolCall("m") {
		t.Error("SupportsToolCall should be true")
	}
	if !cat.SupportsReasoning("m") {
		t.Error("SupportsReasoning should be true")
	}
	if cat.SupportsVision("unknown") {
		t.Error("SupportsVision(unknown) should be false")
	}
}

func TestLoadUsesFreshCache(t *testing.T) {
	dir := t.TempDir()
	oldRoot := cacheRoot
	cacheRoot = dir
	defer func() { cacheRoot = oldRoot }()

	cacheDir := filepath.Join(dir, defaultCacheDir)
	_ = os.MkdirAll(cacheDir, 0755)

	payload := map[string]any{
		"models": []map[string]any{
			{"id": "cached-model", "shape": "anthropic"},
		},
	}
	data, _ := json.Marshal(payload)
	_ = os.WriteFile(filepath.Join(cacheDir, "catalog.json"), data, 0644)

	cat, err := Load()
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	if _, ok := cat.Lookup("cached-model"); !ok {
		t.Error("expected cached-model from cache")
	}
}

func TestLoadFetchesOnStaleCache(t *testing.T) {
	dir := t.TempDir()
	oldRoot := cacheRoot
	cacheRoot = dir
	defer func() { cacheRoot = oldRoot }()

	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("ETag", `"fresh-etag"`)
		w.Header().Set("Content-Type", "application/json")
		payload := map[string]any{
			"models": []map[string]any{
				{"id": "server-model", "shape": "openai"},
			},
		}
		_ = json.NewEncoder(w).Encode(payload)
	}))
	defer server.Close()

	cacheDir := filepath.Join(dir, defaultCacheDir)
	_ = os.MkdirAll(cacheDir, 0755)
	staleData := []byte(`{"models":[{"id":"stale-model","shape":"anthropic"}]}`)
	stalePath := filepath.Join(cacheDir, "catalog.json")
	_ = os.WriteFile(stalePath, staleData, 0644)
	_ = os.Chtimes(stalePath, time.Now().Add(-48*time.Hour), time.Now().Add(-48*time.Hour))

	// Use mock server so we don't hit the real network.
	cat, err := loadWithClient(server.URL, server.Client())
	if err != nil {
		t.Fatalf("loadWithClient: %v", err)
	}
	// Stale cache should be overwritten by the server response.
	if _, ok := cat.Lookup("server-model"); !ok {
		t.Error("expected server-model from fresh fetch")
	}
	if _, ok := cat.Lookup("stale-model"); ok {
		t.Error("stale-model should have been replaced")
	}
}

func TestLoadNotModified(t *testing.T) {
	dir := t.TempDir()
	oldRoot := cacheRoot
	cacheRoot = dir
	defer func() { cacheRoot = oldRoot }()

	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("If-None-Match") == `"my-etag"` {
			w.WriteHeader(http.StatusNotModified)
			return
		}
		w.Header().Set("ETag", `"my-etag"`)
		w.Header().Set("Content-Type", "application/json")
		payload := map[string]any{
			"models": []map[string]any{
				{"id": "etag-model", "shape": "openai"},
			},
		}
		_ = json.NewEncoder(w).Encode(payload)
	}))
	defer server.Close()

	cacheDir := filepath.Join(dir, defaultCacheDir)
	_ = os.MkdirAll(cacheDir, 0755)
	_ = os.WriteFile(filepath.Join(cacheDir, "catalog.json"), []byte(`{"models":[{"id":"etag-model","shape":"openai"}]}`), 0644)
	_ = os.WriteFile(filepath.Join(cacheDir, "catalog.etag"), []byte(`"my-etag"`), 0644)

	cat, err := loadWithClient(server.URL, server.Client())
	if err != nil {
		t.Fatalf("loadWithClient: %v", err)
	}
	if _, ok := cat.Lookup("etag-model"); !ok {
		t.Error("expected etag-model")
	}
}

func TestLoadNetworkFallback(t *testing.T) {
	dir := t.TempDir()
	oldRoot := cacheRoot
	cacheRoot = dir
	defer func() { cacheRoot = oldRoot }()

	// No cache. Use a server that always fails.
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusInternalServerError)
	}))
	defer server.Close()

	_, err := loadWithClient(server.URL, server.Client())
	if err == nil {
		t.Fatal("expected error when no cache and server returns 500")
	}
}

func TestLoadNetworkFallbackToStaleCache(t *testing.T) {
	dir := t.TempDir()
	oldRoot := cacheRoot
	cacheRoot = dir
	defer func() { cacheRoot = oldRoot }()

	cacheDir := filepath.Join(dir, defaultCacheDir)
	_ = os.MkdirAll(cacheDir, 0755)
	staleData := []byte(`{"models":[{"id":"stale-model","shape":"anthropic"}]}`)
	stalePath := filepath.Join(cacheDir, "catalog.json")
	_ = os.WriteFile(stalePath, staleData, 0644)
	_ = os.Chtimes(stalePath, time.Now().Add(-48*time.Hour), time.Now().Add(-48*time.Hour))

	// Server returns 500, so we should fall back to stale cache.
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusInternalServerError)
	}))
	defer server.Close()

	cat, err := loadWithClient(server.URL, server.Client())
	if err != nil {
		t.Fatalf("loadWithClient: %v", err)
	}
	if _, ok := cat.Lookup("stale-model"); !ok {
		t.Error("expected stale-model from fallback cache")
	}
}

func TestParseMalformedJSON(t *testing.T) {
	_, err := parse([]byte(`not json`))
	if err == nil {
		t.Fatal("expected error for malformed JSON")
	}
}

func TestParseMissingModelsField(t *testing.T) {
	cat, err := parse([]byte(`{}`))
	if err != nil {
		t.Fatalf("parse empty object: %v", err)
	}
	_, ok := cat.Lookup("anything")
	if ok {
		t.Error("expected no models")
	}
}

// compile-time check
var _ = func(c *Catalog) {
	_ = c.Lookup
	_ = c.ShapeFor
	_ = c.ContextWindow
	_ = c.SupportsVision
	_ = c.SupportsToolCall
	_ = c.SupportsReasoning
}
