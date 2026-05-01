package agent

import (
	"context"
	"fmt"
	"log/slog"
	"sync"
	"time"

	"github.com/oklog/ulid/v2"

	"mew/internal/hooks"
	"mew/internal/message"
	"mew/internal/provider"
	"mew/internal/session"
	"mew/internal/tools"
)

// Agent is the core conversation loop.
type Agent struct {
	provider   provider.Provider
	dispatcher hooks.Dispatcher
	session    *session.Writer
	tools      map[string]tools.Tool
	mu         sync.Mutex
	messages   []message.Message
	sessionID  string
	system     string
}

// New creates a new agent.
func New(p provider.Provider, d hooks.Dispatcher, s *session.Writer, sessionID string, ts []tools.Tool) *Agent {
	if sessionID == "" {
		sessionID = ulid.MustNew(ulid.Now(), nil).String()
	}
	tm := make(map[string]tools.Tool, len(ts))
	for _, t := range ts {
		tm[t.Name()] = t
	}
	return &Agent{
		provider:   p,
		dispatcher: d,
		session:    s,
		tools:      tm,
		sessionID:  sessionID,
	}
}

// SessionID returns the agent's session identifier.
func (a *Agent) SessionID() string { return a.sessionID }

// SetSystem sets the system prompt prepended to every provider request.
func (a *Agent) SetSystem(s string) { a.system = s }

// Run executes a single turn: sends messages to the provider, handles the
// response stream, executes any tool calls, and loops until the assistant
// stops. Events are sent to the provided channel.
func (a *Agent) Run(ctx context.Context, prompt string, evCh chan<- Event) error {
	userMsg := message.Message{
		ID:        ulid.MustNew(ulid.Now(), nil).String(),
		SessionID: a.sessionID,
		Role:      message.RoleUser,
		Parts: message.Parts{
			message.NewTextPart(ulid.MustNew(ulid.Now(), nil).String(), "", "", prompt, false),
		},
		Time: message.Time{Created: time.Now().UnixMilli()},
	}
	a.appendMessage(userMsg)

	return a.loop(ctx, evCh)
}

func (a *Agent) appendMessage(msg message.Message) {
	a.mu.Lock()
	a.messages = append(a.messages, msg)
	a.mu.Unlock()
	if a.session != nil {
		if err := a.session.WriteMessage(msg); err != nil {
			slog.Error("session write failed", "err", err)
		}
	}
}

func (a *Agent) loop(ctx context.Context, evCh chan<- Event) error {
	for {
		var toolDefs []provider.ToolDef
		for _, t := range a.tools {
			toolDefs = append(toolDefs, provider.ToolDef{
				Name:        t.Name(),
				Description: t.Description(),
				Schema:      t.Schema(),
			})
		}

		req := provider.Request{
			Model:    "",
			Messages: a.messages,
			Tools:    toolDefs,
			System:   a.system,
		}
		_ = a.dispatcher.OnChatParams(ctx, hooks.ChatParams{})
		_ = a.dispatcher.OnChatHeaders(ctx, nil)

		providerEvCh, err := a.provider.Stream(ctx, req)
		if err != nil {
			evCh <- EventError{Err: fmt.Errorf("provider stream: %w", err)}
			return err
		}

		var assistantMsg *message.Message
		for ev := range providerEvCh {
			a.dispatcher.OnEvent(ctx, ev)

			switch e := ev.(type) {
			case provider.EventPartStart:
				if assistantMsg == nil {
					assistantMsg = a.startAssistantMessage()
				}
				assistantMsg.Parts = append(assistantMsg.Parts, e.Part)
				evCh <- EventPartStart{Part: e.Part}

			case provider.EventPartDelta:
				a.applyDelta(assistantMsg, e.PartID, e.Field, e.Delta)
				evCh <- EventPartDelta{PartID: e.PartID, Field: e.Field, Delta: e.Delta}

			case provider.EventPartEnd:
				evCh <- EventPartEnd{PartID: e.PartID}

			case provider.EventMessageEnd:
				if assistantMsg != nil {
					assistantMsg.Time.Completed = time.Now().UnixMilli()
					if assistantMsg.Assistant == nil {
						assistantMsg.Assistant = &message.AssistantMeta{}
					}
					assistantMsg.Assistant.Finish = e.Finish
					assistantMsg.Assistant.Tokens = e.Usage
					assistantMsg.Assistant.Cost = e.Cost
					a.appendMessage(*assistantMsg)
				}
				evCh <- EventMessageEnd{Finish: e.Finish, Usage: e.Usage, Cost: e.Cost}

			case provider.EventError:
				if assistantMsg != nil {
					assistantMsg.Time.Completed = time.Now().UnixMilli()
					if assistantMsg.Assistant == nil {
						assistantMsg.Assistant = &message.AssistantMeta{}
					}
					assistantMsg.Assistant.Error = &message.Error{Kind: "api", Message: e.Err.Error()}
					a.appendMessage(*assistantMsg)
				}
				evCh <- EventError{Err: e.Err}
				return e.Err
			}
		}

		if assistantMsg == nil {
			return fmt.Errorf("no assistant message received")
		}

		if ctx.Err() != nil {
			// Abort: context was cancelled mid-stream.
			assistantMsg.Time.Completed = time.Now().UnixMilli()
			if assistantMsg.Assistant == nil {
				assistantMsg.Assistant = &message.AssistantMeta{}
			}
			assistantMsg.Assistant.Error = &message.Error{Kind: "aborted", Message: ctx.Err().Error()}
			a.appendMessage(*assistantMsg)
			evCh <- EventError{Err: fmt.Errorf("aborted: %w", ctx.Err())}
			return ctx.Err()
		}

		pending := a.pendingToolCalls(assistantMsg)
		if len(pending) == 0 {
			return nil
		}

		results := make(message.Parts, 0, len(pending))
		for _, tc := range pending {
			tc.State.Status = message.ToolRunning
			tc.State.Time.Start = time.Now().UnixMilli()
			a.updateToolCall(assistantMsg, tc.PartID(), tc.State)
			evCh <- EventPartUpdated{PartID: tc.PartID(), Part: tc}

			tool, ok := a.tools[tc.ToolName]
			if !ok {
				tc.State.Status = message.ToolError
				tc.State.Error = fmt.Sprintf("unknown tool %q", tc.ToolName)
				tc.State.Time.End = time.Now().UnixMilli()
				a.updateToolCall(assistantMsg, tc.PartID(), tc.State)
				evCh <- EventPartUpdated{PartID: tc.PartID(), Part: tc}
				results = append(results, message.NewToolResultPart(
					ulid.MustNew(ulid.Now(), nil).String(), "", "", tc.CallID,
				))
				continue
			}

			input := a.dispatcher.OnToolExecuteBefore(ctx, hooks.ToolCall{
				ToolName: tc.ToolName,
				CallID:   tc.CallID,
				Input:    tc.State.Input,
			}, tc.State.Input)

			res, err := tool.Execute(ctx, input)
			out := a.dispatcher.OnToolExecuteAfter(ctx, hooks.ToolCall{
				ToolName: tc.ToolName,
				CallID:   tc.CallID,
				Input:    tc.State.Input,
			}, hooks.ToolOutput{Output: res.Output, Error: res.Error})

			if err != nil || out.Error != "" {
				tc.State.Status = message.ToolError
				if err != nil {
					tc.State.Error = err.Error()
				} else {
					tc.State.Error = out.Error
				}
			} else {
				tc.State.Status = message.ToolCompleted
				tc.State.Output = out.Output
			}
			tc.State.Time.End = time.Now().UnixMilli()
			a.updateToolCall(assistantMsg, tc.PartID(), tc.State)
			evCh <- EventPartUpdated{PartID: tc.PartID(), Part: tc}

			results = append(results, message.NewToolResultPart(
				ulid.MustNew(ulid.Now(), nil).String(), "", "", tc.CallID,
			))
		}

		resultMsg := message.Message{
			ID:        ulid.MustNew(ulid.Now(), nil).String(),
			SessionID: a.sessionID,
			Role:      message.RoleUser,
			Parts:     results,
			Time:      message.Time{Created: time.Now().UnixMilli()},
		}
		a.appendMessage(resultMsg)
	}
}

func (a *Agent) startAssistantMessage() *message.Message {
	return &message.Message{
		ID:        ulid.MustNew(ulid.Now(), nil).String(),
		SessionID: a.sessionID,
		Role:      message.RoleAssistant,
		Parts:     nil,
		Time:      message.Time{Created: time.Now().UnixMilli()},
		Assistant: &message.AssistantMeta{},
	}
}

func (a *Agent) applyDelta(msg *message.Message, partID, field, delta string) {
	for i, p := range msg.Parts {
		if p.PartID() != partID {
			continue
		}
		switch pt := p.(type) {
		case *message.TextPart:
			if field == "text" || field == "" {
				msg.Parts[i] = pt.WithText(delta)
			}
		case *message.ReasoningPart:
			if field == "text" || field == "" {
				msg.Parts[i] = pt.WithText(delta)
			}
		}
	}
}

func (a *Agent) pendingToolCalls(msg *message.Message) []*message.ToolCallPart {
	var out []*message.ToolCallPart
	for _, p := range msg.Parts {
		if tc, ok := p.(*message.ToolCallPart); ok {
			if tc.State.Status == message.ToolPending || tc.State.Status == message.ToolRunning {
				out = append(out, tc)
			}
		}
	}
	return out
}

func (a *Agent) updateToolCall(msg *message.Message, partID string, state message.ToolState) {
	for i, p := range msg.Parts {
		if p.PartID() == partID {
			if tc, ok := p.(*message.ToolCallPart); ok {
				msg.Parts[i] = tc.WithState(state)
			}
			break
		}
	}
}
