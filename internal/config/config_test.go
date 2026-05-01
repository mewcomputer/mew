package config

import (
	"os"
	"path/filepath"
	"testing"
)

func configDir(t *testing.T) string {
	t.Helper()
	dir, err := os.UserConfigDir()
	if err != nil {
		t.Fatalf("user config dir: %v", err)
	}
	return filepath.Join(dir, "mew")
}

func TestLoadMissingConfig(t *testing.T) {
	// Ensure no config file exists at the standard location.
	cfgDir := configDir(t)
	_ = os.RemoveAll(filepath.Join(cfgDir, "config.json"))

	cfg, err := Load()
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	if cfg.Providers == nil {
		t.Fatal("expected non-nil providers map")
	}
	if len(cfg.Providers) != 0 {
		t.Fatalf("expected empty providers, got %d", len(cfg.Providers))
	}
}

func TestLoadExistingConfig(t *testing.T) {
	cfgDir := configDir(t)
	if err := os.MkdirAll(cfgDir, 0750); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	data := []byte(`{"providers":{"opencode-zen":{"shape":"openai","baseURL":"https://example.com","credentialRef":"opencode-zen"}},"defaultModel":"opencode-zen/gpt-4"}`)
	path := filepath.Join(cfgDir, "config.json")
	if err := os.WriteFile(path, data, 0640); err != nil {
		t.Fatalf("write: %v", err)
	}
	defer os.Remove(path)

	cfg, err := Load()
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	if len(cfg.Providers) != 1 {
		t.Fatalf("expected 1 provider, got %d", len(cfg.Providers))
	}
	if cfg.DefaultModel != "opencode-zen/gpt-4" {
		t.Fatalf("expected default model, got %q", cfg.DefaultModel)
	}
}

func TestGetCredentialEnv(t *testing.T) {
	os.Setenv("MEW_CRED_TEST", "secret123")
	defer os.Unsetenv("MEW_CRED_TEST")

	v, err := GetCredential("test")
	if err != nil {
		t.Fatalf("get credential: %v", err)
	}
	if v != "secret123" {
		t.Fatalf("expected secret123, got %q", v)
	}
}

func TestGetCredentialMissing(t *testing.T) {
	_, err := GetCredential("nonexistent-provider-12345")
	if err == nil {
		t.Fatal("expected error for missing credential")
	}
}
