package main

import (
	"strings"
	"testing"

	"mew/internal/config"
)

func TestModelRouting(t *testing.T) {
	cfg := &config.Config{
		Providers: map[string]config.ProviderConfig{
			"custom": {Shape: "openai", BaseURL: "https://example.com"},
		},
	}

	tests := []struct {
		name         string
		providerFlag string
		modelFlag    string
		wantProvider string
		wantModel    string
	}{
		{
			name:         "model with slash auto-routes to known provider",
			providerFlag: "opencode-zen",
			modelFlag:    "opencode-go/kimi-k2.6",
			wantProvider: "opencode-go",
			wantModel:    "kimi-k2.6",
		},
		{
			name:         "model with slash auto-routes to config provider",
			providerFlag: "opencode-zen",
			modelFlag:    "custom/my-model",
			wantProvider: "custom",
			wantModel:    "my-model",
		},
		{
			name:         "explicit provider ignored",
			providerFlag: "opencode-go",
			modelFlag:    "opencode-go/kimi-k2.6",
			wantProvider: "opencode-go",
			wantModel:    "kimi-k2.6",
		},
		{
			name:         "plain model no routing",
			providerFlag: "opencode-zen",
			modelFlag:    "deepseek-v4-flash",
			wantProvider: "opencode-zen",
			wantModel:    "deepseek-v4-flash",
		},
		{
			name:         "unknown prefix passed through",
			providerFlag: "opencode-zen",
			modelFlag:    "z.ai/glm-4.6",
			wantProvider: "opencode-zen",
			wantModel:    "z.ai/glm-4.6",
		},
		{
			name:         "empty model",
			providerFlag: "opencode-zen",
			modelFlag:    "",
			wantProvider: "opencode-zen",
			wantModel:    "",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			providerID := tt.providerFlag
			modelID := tt.modelFlag
			if modelID != "" {
				if idx := strings.Index(modelID, "/"); idx > 0 {
					candidate := modelID[:idx]
					if isKnownProvider(cfg, candidate) {
						if providerID == "opencode-zen" {
							providerID = candidate
						}
						modelID = modelID[idx+1:]
					}
				}
			}
			if providerID != tt.wantProvider {
				t.Fatalf("provider: got %q, want %q", providerID, tt.wantProvider)
			}
			if modelID != tt.wantModel {
				t.Fatalf("model: got %q, want %q", modelID, tt.wantModel)
			}
		})
	}
}
