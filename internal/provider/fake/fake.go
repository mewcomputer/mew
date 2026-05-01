package fake

import (
	"context"

	"mew/internal/message"
	"mew/internal/provider"
)

// Provider is a fake provider for testing.
type Provider struct {
	name     string
	script   []provider.Event
	callHook func(req provider.Request)
}

// New creates a fake provider with a scripted event stream.
func New(name string, script []provider.Event) *Provider {
	return &Provider{name: name, script: script}
}

// NewWithHook creates a fake provider that also invokes a hook on each request.
func NewWithHook(name string, script []provider.Event, hook func(req provider.Request)) *Provider {
	return &Provider{name: name, script: script, callHook: hook}
}

func (p *Provider) Name() string { return p.name }

func (p *Provider) Stream(ctx context.Context, req provider.Request) (<-chan provider.Event, error) {
	if p.callHook != nil {
		p.callHook(req)
	}
	evCh := make(chan provider.Event)
	go func() {
		defer close(evCh)
		for _, ev := range p.script {
			select {
			case <-ctx.Done():
				return
			case evCh <- ev:
			}
		}
	}()
	return evCh, nil
}

// Script helpers.

func TextPartStart(id string) provider.Event {
	return provider.EventPartStart{Part: message.NewTextPart(id, "", "", "", false)}
}

func TextDelta(id, delta string) provider.Event {
	return provider.EventPartDelta{PartID: id, Field: "text", Delta: delta}
}

func PartEnd(id string) provider.Event {
	return provider.EventPartEnd{PartID: id}
}

func MessageEnd(finish string) provider.Event {
	return provider.EventMessageEnd{Finish: finish}
}

func ToolCallPartStart(id, toolName, callID string, input map[string]any) provider.Event {
	return provider.EventPartStart{
		Part: message.NewToolCallPart(id, "", "", toolName, callID, message.ToolState{
			Status: message.ToolPending,
			Input:  input,
		}),
	}
}
