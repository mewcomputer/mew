package hooks

import (
	"context"
	"net/http"

	"mew/internal/message"
)

// ChatParams holds mutable generation parameters.
type ChatParams struct {
	Temperature *float64
	TopP        *float64
	MaxTokens   *int
}

// ToolCall is a snapshot of a tool invocation passed to hooks.
type ToolCall struct {
	ToolName string
	CallID   string
	Input    map[string]any
}

// ToolOutput is the result of a tool execution passed to hooks.
type ToolOutput struct {
	Output string
	Error  string
}

// PermissionDecision is the outcome of a permission check.
type PermissionDecision string

const (
	AllowOnce    PermissionDecision = "allow_once"
	AllowSession PermissionDecision = "allow_session"
	Deny         PermissionDecision = "deny"
	Prompt       PermissionDecision = "prompt"
)

// Dispatcher is the hook point interface wired through the agent loop.
type Dispatcher interface {
	// OnEvent is observe-only. Errors are logged, never propagated.
	// The concrete type is agent.Event; using any here avoids an import cycle.
	OnEvent(ctx context.Context, ev any)

	// Mutating hooks. Each returns the (possibly modified) value.
	// Errors fall back to the input unchanged and are logged.
	OnChatMessage(ctx context.Context, msg message.Message) message.Message
	OnChatParams(ctx context.Context, p ChatParams) ChatParams
	OnChatHeaders(ctx context.Context, h http.Header) http.Header
	OnToolExecuteBefore(ctx context.Context, call ToolCall, input map[string]any) map[string]any
	OnToolExecuteAfter(ctx context.Context, call ToolCall, output ToolOutput) ToolOutput
	OnPermissionAsk(ctx context.Context, call ToolCall, current PermissionDecision) PermissionDecision
	OnShellEnv(ctx context.Context, env map[string]string) map[string]string
}

// nopDispatcher is the default no-op implementation used through m6.
type nopDispatcher struct{}

func (nopDispatcher) OnEvent(ctx context.Context, ev any)                                     {}
func (nopDispatcher) OnChatMessage(ctx context.Context, msg message.Message) message.Message    { return msg }
func (nopDispatcher) OnChatParams(ctx context.Context, p ChatParams) ChatParams                 { return p }
func (nopDispatcher) OnChatHeaders(ctx context.Context, h http.Header) http.Header              { return h }
func (nopDispatcher) OnToolExecuteBefore(ctx context.Context, call ToolCall, input map[string]any) map[string]any {
	return input
}
func (nopDispatcher) OnToolExecuteAfter(ctx context.Context, call ToolCall, output ToolOutput) ToolOutput {
	return output
}
func (nopDispatcher) OnPermissionAsk(ctx context.Context, call ToolCall, current PermissionDecision) PermissionDecision {
	return current
}
func (nopDispatcher) OnShellEnv(ctx context.Context, env map[string]string) map[string]string { return env }

// Nop returns the shared no-op dispatcher.
func Nop() Dispatcher { return nopDispatcher{} }
