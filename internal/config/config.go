package config

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// Config is the top-level user configuration.
type Config struct {
	Providers    map[string]ProviderConfig `json:"providers"`
	DefaultModel string                    `json:"defaultModel"`
}

// ProviderConfig describes a single provider entry.
type ProviderConfig struct {
	Shape         string `json:"shape"`
	BaseURL       string `json:"baseURL"`
	CredentialRef string `json:"credentialRef"`
}

// Load reads config from the standard location.
func Load() (*Config, error) {
	dir, err := os.UserConfigDir()
	if err != nil {
		return nil, err
	}
	path := filepath.Join(dir, "mew", "config.json")
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return &Config{Providers: make(map[string]ProviderConfig)}, nil
		}
		return nil, err
	}
	var cfg Config
	if err := json.Unmarshal(data, &cfg); err != nil {
		return nil, fmt.Errorf("config.json: %w", err)
	}
	if cfg.Providers == nil {
		cfg.Providers = make(map[string]ProviderConfig)
	}
	return &cfg, nil
}

// GetCredential resolves a credential reference.
// Order: env var MEW_CRED_<REF_NORMALIZED>, then credentials.json fallback.
// The ref is uppercased and non-alphanumerics are replaced with underscores
// so that "opencode-zen" maps to MEW_CRED_OPENCODE_ZEN.
func GetCredential(ref string) (string, error) {
	normalized := strings.ToUpper(ref)
	normalized = strings.Map(func(r rune) rune {
		if (r >= 'a' && r <= 'z') || (r >= 'A' && r <= 'Z') || (r >= '0' && r <= '9') {
			return r
		}
		return '_'
	}, normalized)
	envKey := "MEW_CRED_" + normalized
	if v := os.Getenv(envKey); v != "" {
		return v, nil
	}

	dir, err := os.UserConfigDir()
	if err != nil {
		return "", err
	}
	path := filepath.Join(dir, "mew", "credentials.json")
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return "", fmt.Errorf("credential %q not found in env or credentials.json", ref)
		}
		return "", err
	}
	var creds map[string]string
	if err := json.Unmarshal(data, &creds); err != nil {
		return "", err
	}
	v, ok := creds[ref]
	if !ok {
		return "", fmt.Errorf("credential %q not found", ref)
	}
	return v, nil
}
