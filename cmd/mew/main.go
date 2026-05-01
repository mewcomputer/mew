package main

import (
	"context"
	"flag"
	"fmt"
	"log/slog"
	"os"
	"strings"

	"github.com/oklog/ulid/v2"

	"mew/internal/agent"
	"mew/internal/config"
	mewcontext "mew/internal/context"
	"mew/internal/hooks"
	"mew/internal/message"
	"mew/internal/provider"
	"mew/internal/provider/anthropic"
	"mew/internal/provider/openai"
	"mew/internal/session"
	"mew/internal/tools"
)

func main() {
	if err := run(); err != nil {
		slog.Error("fatal", "err", err)
		os.Exit(1)
	}
}

func run() error {
	if len(os.Args) < 2 {
		return fmt.Errorf("usage: mew run \"<prompt>\"")
	}

	switch os.Args[1] {
	case "run":
		return runCmd(os.Args[2:])
	default:
		return fmt.Errorf("unknown command: %s", os.Args[1])
	}
}

func runCmd(args []string) error {
	fs := flag.NewFlagSet("run", flag.ExitOnError)
	providerFlag := fs.String("provider", "opencode-zen", "provider ID")
	modelFlag := fs.String("model", "", "model ID (overrides provider default)")
	rawFlag := fs.Bool("raw", false, "dump raw request/response to stderr")
	if err := fs.Parse(args); err != nil {
		return err
	}

	prompt := strings.Join(fs.Args(), " ")
	if prompt == "" {
		return fmt.Errorf("missing prompt")
	}

	cfg, err := config.Load()
	if err != nil {
		return fmt.Errorf("load config: %w", err)
	}

	// Model IDs like "opencode-go/kimi-k2.6" auto-route to that provider
	// only if the prefix matches a known provider. Otherwise pass the full
	// string through (e.g. "z.ai/glm-4.6" stays intact).
	modelID := *modelFlag
	providerID := *providerFlag
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

	p, err := buildProvider(cfg, providerID, modelID)
	if err != nil {
		return fmt.Errorf("build provider: %w", err)
	}
	if *rawFlag {
		if d, ok := p.(interface{ SetDump(bool) }); ok {
			d.SetDump(true)
		}
	}

	sessionID := ulid.MustNew(ulid.Now(), nil).String()
	sw, err := session.Open(sessionID)
	if err != nil {
		return fmt.Errorf("open session: %w", err)
	}
	defer sw.Close()

	a := agent.New(p, hooks.Nop(), sw, sessionID, []tools.Tool{tools.Echo{}})

	// Load project context files and prepend to system prompt.
	ctxLoader := mewcontext.NewLoader("")
	if cwd, err := os.Getwd(); err == nil {
		ctxLoader = mewcontext.NewLoader(cwd)
	}
	ctxFiles, _ := ctxLoader.Load()
	if len(ctxFiles) > 0 {
		a.SetSystem(mewcontext.BuildSystemPrompt(ctxFiles))
	}

	ctx := context.Background()
	evCh := make(chan agent.Event)
	go func() {
		defer close(evCh)
		if err := a.Run(ctx, prompt, evCh); err != nil {
			slog.Error("agent run failed", "err", err)
		}
	}()

	partTypes := make(map[string]string)
	for ev := range evCh {
		switch e := ev.(type) {
		case agent.EventPartStart:
			switch p := e.Part.(type) {
			case *message.TextPart:
				partTypes[p.PartID()] = "text"
			case *message.ReasoningPart:
				partTypes[p.PartID()] = "reasoning"
				fmt.Fprint(os.Stderr, "\n[thinking]\n")
			}
		case agent.EventPartDelta:
			if partTypes[e.PartID] == "reasoning" {
				fmt.Fprint(os.Stderr, e.Delta)
			} else {
				fmt.Print(e.Delta)
			}
		case agent.EventPartEnd:
			if partTypes[e.PartID] == "reasoning" {
				fmt.Fprint(os.Stderr, "\n[/thinking]\n")
			}
			delete(partTypes, e.PartID)
		case agent.EventMessageEnd:
			if e.Finish == "stop" {
				fmt.Println()
			}
		case agent.EventPartUpdated:
			if tc, ok := e.Part.(*message.ToolCallPart); ok {
				switch tc.State.Status {
				case message.ToolRunning:
					fmt.Fprintf(os.Stderr, "\n[tool: %s]\n", tc.ToolName)
				case message.ToolCompleted:
					fmt.Fprintf(os.Stderr, "[tool completed: %s]\n", tc.ToolName)
				case message.ToolError:
					fmt.Fprintf(os.Stderr, "[tool error: %s] %s\n", tc.ToolName, tc.State.Error)
				}
			}
		case agent.EventError:
			return e.Err
		}
	}

	return nil
}

// isKnownProvider returns true if the provider ID exists in config or is a
// built-in fallback.
func isKnownProvider(cfg *config.Config, providerID string) bool {
	if _, ok := cfg.Providers[providerID]; ok {
		return true
	}
	switch providerID {
	case "opencode-zen", "opencode-go", "z-ai":
		return true
	}
	return false
}

func buildProvider(cfg *config.Config, providerID, modelOverride string) (provider.Provider, error) {
	pc, ok := cfg.Providers[providerID]
	if !ok {
		// Fall back to built-in defaults for known providers.
		switch providerID {
		case "opencode-zen":
			pc = config.ProviderConfig{
				Shape:         "openai",
				BaseURL:       "https://opencode.ai/zen/v1",
				CredentialRef: "opencode-zen",
			}
		case "opencode-go":
			pc = config.ProviderConfig{
				Shape:         "openai",
				BaseURL:       "https://opencode.ai/zen/go/v1",
				CredentialRef: "opencode-zen",
			}
		case "z-ai":
			pc = config.ProviderConfig{
				Shape:         "anthropic",
				BaseURL:       "https://api.z.ai/api/anthropic",
				CredentialRef: "z-ai",
			}
		default:
			return nil, fmt.Errorf("unknown provider %q", providerID)
		}
	}

	creds, err := config.GetCredential(pc.CredentialRef)
	if err != nil {
		return nil, fmt.Errorf("get credential: %w", err)
	}

	model := modelOverride
	if model == "" {
		model = cfg.DefaultModel
		if model == "" {
			model = "deepseek-v4-flash" // generic fallback
		}
	}

	// Opencode-go hosts both OpenAI-shape and Anthropic-shape models.
	// Auto-route minimax models to the anthropic endpoint.
	shape := pc.Shape
	baseURL := pc.BaseURL
	if providerID == "opencode-go" && strings.HasPrefix(model, "minimax-") {
		shape = "anthropic"
		baseURL = "https://opencode.ai/zen/go/v1"
	}

	switch shape {
	case "openai":
		return openai.New(providerID, baseURL, model, creds), nil
	case "anthropic":
		return anthropic.New(providerID, baseURL, model, creds), nil
	default:
		return nil, fmt.Errorf("unsupported shape %q for provider %q", shape, providerID)
	}
}
