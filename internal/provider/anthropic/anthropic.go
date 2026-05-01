package anthropic

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strings"
	"time"

	"github.com/oklog/ulid/v2"

	"mew/internal/message"
	"mew/internal/provider"
)

// Adapter implements provider.Provider for the Anthropic Messages API.
type Adapter struct {
	name    string
	baseURL string
	model   string
	apiKey  string
	client  *http.Client
}

// New creates a new Anthropic-shape adapter.
func New(name, baseURL, model, apiKey string) *Adapter {
	return &Adapter{
		name:    name,
		baseURL: strings.TrimSuffix(baseURL, "/"),
		model:   model,
		apiKey:  apiKey,
		client:  &http.Client{Timeout: 120 * time.Second},
	}
}

func (a *Adapter) Name() string { return a.name }

func (a *Adapter) Stream(ctx context.Context, req provider.Request) (<-chan provider.Event, error) {
	body, err := a.buildRequestBody(req)
	if err != nil {
		return nil, err
	}

	hreq, err := http.NewRequestWithContext(ctx, "POST", a.baseURL+"/messages", bytes.NewReader(body))
	if err != nil {
		return nil, err
	}
	hreq.Header.Set("Content-Type", "application/json")
	hreq.Header.Set("X-API-Key", a.apiKey)
	hreq.Header.Set("Anthropic-Version", "2023-06-01")
	hreq.Header.Set("Accept", "text/event-stream")

	resp, err := a.client.Do(hreq)
	if err != nil {
		return nil, fmt.Errorf("http request: %w", err)
	}

	if resp.StatusCode != http.StatusOK {
		data, _ := io.ReadAll(resp.Body)
		resp.Body.Close()
		return nil, fmt.Errorf("http %d: %s", resp.StatusCode, string(data))
	}

	evCh := make(chan provider.Event)
	go a.readStream(resp.Body, evCh)
	return evCh, nil
}

func (a *Adapter) buildRequestBody(req provider.Request) ([]byte, error) {
	var messages []map[string]any
	for _, m := range req.Messages {
		msg := a.buildWireMessage(req.Messages, m)
		if msg != nil {
			messages = append(messages, msg)
		}
	}

	body := map[string]any{
		"model":      a.model,
		"max_tokens": 4096,
		"messages":   messages,
		"stream":     true,
	}

	if req.System != "" {
		body["system"] = req.System
	}

	if len(req.Tools) > 0 {
		var tools []map[string]any
		for _, t := range req.Tools {
			tools = append(tools, map[string]any{
				"name":        t.Name,
				"description": t.Description,
				"input_schema": json.RawMessage(t.Schema),
			})
		}
		body["tools"] = tools
	}

	return json.Marshal(body)
}

func (a *Adapter) buildWireMessage(all []message.Message, m message.Message) map[string]any {
	var content []map[string]any

	switch m.Role {
	case message.RoleUser:
		for _, p := range m.Parts {
			switch pt := p.(type) {
			case *message.TextPart:
				content = append(content, map[string]any{
					"type": "text",
					"text": pt.Text,
				})
			case *message.ToolResultPart:
				output := a.findToolOutput(all, pt.CallID)
				content = append(content, map[string]any{
					"type":      "tool_result",
					"tool_use_id": pt.CallID,
					"content":   output,
				})
			case *message.FilePart:
				if strings.HasPrefix(pt.Mime, "image/") {
					content = append(content, map[string]any{
						"type": "image",
						"source": map[string]any{
							"type":      "base64",
							"media_type": pt.Mime,
							"data":      a.readImageData(pt.URL),
						},
					})
				} else {
					content = append(content, map[string]any{
						"type": "text",
						"text": fmt.Sprintf("[File: %s]", pt.Filename),
					})
				}
			}
		}
		if len(content) == 0 {
			return nil
		}
		return map[string]any{"role": "user", "content": content}

	case message.RoleAssistant:
		for _, p := range m.Parts {
			switch pt := p.(type) {
			case *message.TextPart:
				content = append(content, map[string]any{
					"type": "text",
					"text": pt.Text,
				})
			case *message.ReasoningPart:
				block := map[string]any{
					"type":     "thinking",
					"thinking": pt.Text,
				}
				if pt.Signature != "" {
					block["signature"] = pt.Signature
				}
				content = append(content, block)
			case *message.ToolCallPart:
				content = append(content, map[string]any{
					"type":  "tool_use",
					"id":    pt.CallID,
					"name":  pt.ToolName,
					"input": pt.State.Input,
				})
			}
		}
		if len(content) == 0 {
			return nil
		}
		return map[string]any{"role": "assistant", "content": content}
	}

	return nil
}

func (a *Adapter) findToolOutput(all []message.Message, callID string) string {
	for _, m := range all {
		for _, p := range m.Parts {
			if tc, ok := p.(*message.ToolCallPart); ok && tc.CallID == callID {
				return tc.State.Output
			}
		}
	}
	return ""
}

func (a *Adapter) readImageData(url string) string {
	// TODO: implement image reading for M1
	return ""
}

func (a *Adapter) readStream(body io.ReadCloser, evCh chan<- provider.Event) {
	defer close(evCh)
	defer body.Close()

	scanner := bufio.NewScanner(body)
	scanner.Split(scanLines)

	var currentEvent string
	var currentTextPart *message.TextPart
	var currentReasoningPart *message.ReasoningPart
	var currentToolCall *toolCallAccumulator

	for scanner.Scan() {
		line := scanner.Text()
		if strings.HasPrefix(line, "event: ") {
			currentEvent = strings.TrimPrefix(line, "event: ")
			continue
		}

		if !strings.HasPrefix(line, "data: ") {
			continue
		}
		data := strings.TrimPrefix(line, "data: ")

		switch currentEvent {
		case "message_start":
			// Start of message; nothing to emit yet.

		case "content_block_start":
			a.handleContentBlockStart(data, evCh, &currentTextPart, &currentReasoningPart, &currentToolCall)

		case "content_block_delta":
			a.handleContentBlockDelta(data, evCh, currentTextPart, currentReasoningPart, currentToolCall)

		case "content_block_stop":
			a.handleContentBlockStop(evCh, &currentTextPart, &currentReasoningPart, &currentToolCall)

		case "message_delta":
			a.handleMessageDelta(data, evCh)

		case "message_stop":
			// message_delta already emitted EventMessageEnd with the real
			// stop_reason and usage. message_stop is just a sentinel.
			break

		case "error":
			var errResp struct {
				Error struct {
					Type    string `json:"type"`
					Message string `json:"message"`
				} `json:"error"`
			}
			json.Unmarshal([]byte(data), &errResp)
			evCh <- provider.EventError{Err: fmt.Errorf("anthropic error: %s", errResp.Error.Message)}
		}
	}

	if err := scanner.Err(); err != nil {
		evCh <- provider.EventError{Err: fmt.Errorf("sse stream: %w", err)}
	}
}

func (a *Adapter) handleContentBlockStart(data string, evCh chan<- provider.Event,
	currentTextPart **message.TextPart, currentReasoningPart **message.ReasoningPart,
	currentToolCall **toolCallAccumulator) {

	var event struct {
		ContentBlock struct {
			Type string `json:"type"`
			ID   string `json:"id"`
			Name string `json:"name"`
		} `json:"content_block"`
	}
	if err := json.Unmarshal([]byte(data), &event); err != nil {
		return
	}

	switch event.ContentBlock.Type {
	case "text":
		*currentTextPart = message.NewTextPart(ulid.MustNew(ulid.Now(), nil).String(), "", "", "", false)
		evCh <- provider.EventPartStart{Part: *currentTextPart}
	case "thinking":
		*currentReasoningPart = message.NewReasoningPart(ulid.MustNew(ulid.Now(), nil).String(), "", "", "", "")
		evCh <- provider.EventPartStart{Part: *currentReasoningPart}
	case "tool_use":
		part := message.NewToolCallPart(
			ulid.MustNew(ulid.Now(), nil).String(),
			"", "",
			event.ContentBlock.Name,
			event.ContentBlock.ID,
			message.ToolState{Status: message.ToolPending, Time: message.ToolTime{Start: time.Now().UnixMilli()}},
		)
		*currentToolCall = &toolCallAccumulator{part: part}
		evCh <- provider.EventPartStart{Part: part}
	}
}

func (a *Adapter) handleContentBlockDelta(data string, evCh chan<- provider.Event,
	currentTextPart *message.TextPart, currentReasoningPart *message.ReasoningPart,
	currentToolCall *toolCallAccumulator) {

	var event struct {
		Delta struct {
			Type        string `json:"type"`
			Text        string `json:"text,omitempty"`
			PartialJSON string `json:"partial_json,omitempty"`
			Thinking    string `json:"thinking,omitempty"`
		} `json:"delta"`
	}
	if err := json.Unmarshal([]byte(data), &event); err != nil {
		return
	}

	switch event.Delta.Type {
	case "text_delta":
		if currentTextPart != nil {
			evCh <- provider.EventPartDelta{
				PartID: currentTextPart.PartID(),
				Field:  "text",
				Delta:  event.Delta.Text,
			}
		}
	case "input_json_delta":
		if currentToolCall != nil {
			currentToolCall.json += event.Delta.PartialJSON
			evCh <- provider.EventPartDelta{
				PartID: currentToolCall.part.PartID(),
				Field:  "arguments",
				Delta:  event.Delta.PartialJSON,
			}
		}
	case "thinking_delta":
		if currentReasoningPart != nil {
			evCh <- provider.EventPartDelta{
				PartID: currentReasoningPart.PartID(),
				Field:  "text",
				Delta:  event.Delta.Thinking,
			}
		}
	}
}

func (a *Adapter) handleContentBlockStop(evCh chan<- provider.Event,
	currentTextPart **message.TextPart, currentReasoningPart **message.ReasoningPart,
	currentToolCall **toolCallAccumulator) {

	if *currentTextPart != nil {
		evCh <- provider.EventPartEnd{PartID: (*currentTextPart).PartID()}
		*currentTextPart = nil
	}
	if *currentReasoningPart != nil {
		evCh <- provider.EventPartEnd{PartID: (*currentReasoningPart).PartID()}
		*currentReasoningPart = nil
	}
	if *currentToolCall != nil {
		(*currentToolCall).finalize()
		evCh <- provider.EventPartEnd{PartID: (*currentToolCall).part.PartID()}
		*currentToolCall = nil
	}
}

func (a *Adapter) handleMessageDelta(data string, evCh chan<- provider.Event) {
	var msgDelta struct {
		Delta struct {
			StopReason string `json:"stop_reason"`
		} `json:"delta"`
		Usage struct {
			InputTokens  int `json:"input_tokens"`
			OutputTokens int `json:"output_tokens"`
		} `json:"usage"`
	}
	if err := json.Unmarshal([]byte(data), &msgDelta); err != nil {
		return
	}

	finish := a.mapFinishReason(msgDelta.Delta.StopReason)
	evCh <- provider.EventMessageEnd{
		Finish: finish,
		Usage: message.Tokens{
			Input:  msgDelta.Usage.InputTokens,
			Output: msgDelta.Usage.OutputTokens,
		},
	}
}

func (a *Adapter) mapFinishReason(reason string) string {
	switch reason {
	case "end_turn":
		return "stop"
	case "max_tokens":
		return "length"
	case "tool_use":
		return "tool_use"
	case "stop_sequence":
		return "stop"
	default:
		return "error"
	}
}

type toolCallAccumulator struct {
	part *message.ToolCallPart
	json string
}

func (acc *toolCallAccumulator) finalize() {
	if acc.json != "" {
		var input map[string]any
		if err := json.Unmarshal([]byte(acc.json), &input); err == nil {
			acc.part.State.Input = input
		}
	}
}

// scanLines handles \r\n, \n, and preserves the line ending.
func scanLines(data []byte, atEOF bool) (advance int, token []byte, err error) {
	if atEOF && len(data) == 0 {
		return 0, nil, nil
	}
	if i := bytes.Index(data, []byte("\n")); i >= 0 {
		return i + 1, dropCR(data[0:i]), nil
	}
	if atEOF {
		return len(data), dropCR(data), nil
	}
	return 0, nil, nil
}

func dropCR(data []byte) []byte {
	if len(data) > 0 && data[len(data)-1] == '\r' {
		return data[0 : len(data)-1]
	}
	return data
}
