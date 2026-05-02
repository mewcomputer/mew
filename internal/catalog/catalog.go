package catalog

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"time"
)

const (
	catalogURL      = "https://models.dev/api.json"
	cacheMaxAge     = 24 * time.Hour
	defaultCacheDir = "mew"
)

// cacheRoot overrides os.UserConfigDir for tests.
var cacheRoot string

// Model describes a single model entry from the models.dev catalog.
type Model struct {
	ID            string   `json:"id"`
	Provider      string   `json:"provider"`
	ContextWindow int      `json:"context_window"`
	MaxOutput     int      `json:"max_output"`
	ToolCall      bool     `json:"tool_call"`
	Reasoning     bool     `json:"reasoning"`
	Vision        bool     `json:"vision"`
	Shape         string   `json:"shape"` // "openai" | "anthropic"
	Pricing       Pricing  `json:"pricing"`
}

// Pricing holds per-token cost info.
type Pricing struct {
	Input        float64 `json:"input"`
	Output       float64 `json:"output"`
	CacheRead    float64 `json:"cache_read"`
	CacheWrite   float64 `json:"cache_write"`
	Reasoning    float64 `json:"reasoning"`
}

// Catalog is the loaded model registry.
type Catalog struct {
	models map[string]Model
}

// Load fetches the catalog, using a local cache when fresh.
func Load() (*Catalog, error) {
	return loadWithClient(catalogURL, http.DefaultClient)
}

func loadWithClient(url string, client *http.Client) (*Catalog, error) {
	var cacheDir string
	if cacheRoot != "" {
		cacheDir = filepath.Join(cacheRoot, defaultCacheDir)
	} else {
		root, err := os.UserConfigDir()
		if err != nil {
			return nil, err
		}
		cacheDir = filepath.Join(root, defaultCacheDir)
	}
	if err := os.MkdirAll(cacheDir, 0755); err != nil {
		return nil, err
	}

	cachePath := filepath.Join(cacheDir, "catalog.json")
	etagPath := filepath.Join(cacheDir, "catalog.etag")

	// Try cached copy first.
	if data, err := os.ReadFile(cachePath); err == nil {
		if fi, err := os.Stat(cachePath); err == nil && time.Since(fi.ModTime()) < cacheMaxAge {
			return parse(data)
		}
	}

	// Fetch fresh copy.
	req, err := http.NewRequest("GET", url, nil)
	if err != nil {
		return nil, err
	}

	if etag, err := os.ReadFile(etagPath); err == nil {
		req.Header.Set("If-None-Match", string(etag))
	}

	resp, err := client.Do(req)
	if err != nil {
		// Network failure: fall back to stale cache if present.
		if data, err := os.ReadFile(cachePath); err == nil {
			return parse(data)
		}
		return nil, fmt.Errorf("fetch catalog: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode == http.StatusNotModified {
		// Cache is still valid; refresh its mtime and use it.
		if data, err := os.ReadFile(cachePath); err == nil {
			_ = os.Chtimes(cachePath, time.Now(), time.Now())
			return parse(data)
		}
	}

	if resp.StatusCode != http.StatusOK {
		// Non-OK: fall back to stale cache.
		if data, err := os.ReadFile(cachePath); err == nil {
			return parse(data)
		}
		return nil, fmt.Errorf("fetch catalog: status %d", resp.StatusCode)
	}

	data, err := io.ReadAll(resp.Body)
	if err != nil {
		if stale, err := os.ReadFile(cachePath); err == nil {
			return parse(stale)
		}
		return nil, fmt.Errorf("read catalog body: %w", err)
	}

	// Write cache and etag.
	_ = os.WriteFile(cachePath, data, 0644)
	if etag := resp.Header.Get("ETag"); etag != "" {
		_ = os.WriteFile(etagPath, []byte(etag), 0644)
	}

	return parse(data)
}

func parse(data []byte) (*Catalog, error) {
	var payload struct {
		Models []Model `json:"models"`
	}
	if err := json.Unmarshal(data, &payload); err != nil {
		return nil, fmt.Errorf("parse catalog: %w", err)
	}

	cat := &Catalog{models: make(map[string]Model, len(payload.Models))}
	for _, m := range payload.Models {
		cat.models[m.ID] = m
	}
	return cat, nil
}

// Lookup returns a model by ID, or zero value if unknown.
func (c *Catalog) Lookup(id string) (Model, bool) {
	m, ok := c.models[id]
	return m, ok
}

// ShapeFor returns the adapter shape for a model ID.
// Falls back to "openai" if the model is unknown.
func (c *Catalog) ShapeFor(id string) string {
	if m, ok := c.models[id]; ok && m.Shape != "" {
		return m.Shape
	}
	return "openai"
}

// ContextWindow returns the context window size for a model ID.
// Falls back to 128k if unknown.
func (c *Catalog) ContextWindow(id string) int {
	if m, ok := c.models[id]; ok && m.ContextWindow > 0 {
		return m.ContextWindow
	}
	return 128000
}

// SupportsVision reports whether a model supports image input.
func (c *Catalog) SupportsVision(id string) bool {
	if m, ok := c.models[id]; ok {
		return m.Vision
	}
	return false
}

// SupportsToolCall reports whether a model supports tool calling.
func (c *Catalog) SupportsToolCall(id string) bool {
	if m, ok := c.models[id]; ok {
		return m.ToolCall
	}
	return false
}

// SupportsReasoning reports whether a model supports reasoning blocks.
func (c *Catalog) SupportsReasoning(id string) bool {
	if m, ok := c.models[id]; ok {
		return m.Reasoning
	}
	return false
}
