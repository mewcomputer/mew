package openai

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"strings"
	"time"

	"github.com/oklog/ulid/v2"

	"mew/internal/message"
	"mew/internal/provider"
	"mew/internal/provider/imageutil"
)

// Adapter implements provider.Provider for the OpenAI chat completions API.
type Adapter struct {
	name      string
	baseURL   string
	model     string
	apiKey    string
	client    *http.Client
	dump      bool // print raw request/response to stderr
}

// New creates a new OpenAI-shape adapter.
func New(name, baseURL, model, apiKey string) *Adapter {
	return &Adapter{
		name:    name,
		baseURL: strings.TrimSuffix(baseURL, "/"),
		model:   model,
		apiKey:  apiKey,
		client:  &http.Client{Timeout: 120 * time.Second},
	}
}

// SetDump enables raw request/response dumping to stderr.
func (a *Adapter) SetDump(v bool) { a.dump = v }

func (a *Adapter) Name() string { return a.name }

func (a *Adapter) Stream(ctx context.Context, req provider.Request) (<-chan provider.Event, error) {
	body, err := a.buildRequestBody(req)
	if err != nil {
		return nil, err
	}

	if a.dump {
		var pretty bytes.Buffer
		if err := json.Indent(&pretty, body, "", "  "); err == nil {
			fmt.Fprintf(os.Stderr, "\n[RAW REQUEST BODY]\n%s\n\n", pretty.String())
		} else {
			fmt.Fprintf(os.Stderr, "\n[RAW REQUEST BODY]\n%s\n\n", string(body))
		}
	}

	hreq, err := http.NewRequestWithContext(ctx, "POST", a.baseURL+"/chat/completions", bytes.NewReader(body))
	if err != nil {
		return nil, err
	}
	hreq.Header.Set("Content-Type", "application/json")
	hreq.Header.Set("Authorization", "Bearer "+a.apiKey)
	hreq.Header.Set("Accept", "text/event-stream")

	policy := provider.DefaultRetryPolicy()
	var resp *http.Response
	for attempt := 0; ; attempt++ {
		resp, err = a.client.Do(hreq.Clone(ctx))
		if err != nil {
			return nil, fmt.Errorf("http request: %w", err)
		}

		if resp.StatusCode == http.StatusOK {
			break
		}

		data, _ := io.ReadAll(resp.Body)
		resp.Body.Close()

		backoff, retry := policy.ShouldRetry(resp.StatusCode, attempt)
		if !retry {
			kind, msg := provider.ClassifyError(resp.StatusCode, string(data))
			return nil, fmt.Errorf("%s: %s", kind, msg)
		}

		select {
		case <-ctx.Done():
			return nil, ctx.Err()
		case <-time.After(backoff):
		}
	}

	evCh := make(chan provider.Event)
	go a.readStream(resp.Body, evCh)
	return evCh, nil
}

func (a *Adapter) findToolOutput(messages []message.Message, callID string) string {
	for _, m := range messages {
		for _, p := range m.Parts {
			if tc, ok := p.(*message.ToolCallPart); ok && tc.CallID == callID {
				return tc.State.Output
			}
		}
	}
	return ""
}

func (a *Adapter) buildRequestBody(req provider.Request) ([]byte, error) {
	var messages []map[string]any
	for _, m := range req.Messages {
		msg := a.buildWireMessage(req.Messages, m)
		messages = append(messages, msg...)
	}

	body := map[string]any{
		"model":    a.model,
		"messages": messages,
		"stream":   true,
	}

	if req.System != "" {
		messages = append([]map[string]any{{"role": "system", "content": req.System}}, messages...)
		body["messages"] = messages
	}

	if len(req.Tools) > 0 {
		var toolDefs []map[string]any
		for _, t := range req.Tools {
			toolDefs = append(toolDefs, map[string]any{
				"type": "function",
				"function": map[string]any{
					"name":        t.Name,
					"description": t.Description,
					"parameters":  json.RawMessage(t.Schema),
				},
			})
		}
		body["tools"] = toolDefs
	}

	return json.Marshal(body)
}

func (a *Adapter) buildWireMessage(all []message.Message, m message.Message) []map[string]any {
	var out []map[string]any

	switch m.Role {
	case message.RoleUser:
		var textContent strings.Builder
		var imageBlocks []map[string]any
		var toolResults []map[string]any
		for _, p := range m.Parts {
			switch pt := p.(type) {
			case *message.TextPart:
				textContent.WriteString(pt.Text)
			case *message.FilePart:
				if strings.HasPrefix(pt.Mime, "image/") {
					mime, b64, err := imageutil.Resolve(pt.URL)
					if err == nil {
						imageBlocks = append(imageBlocks, map[string]any{
							"type": "image_url",
							"image_url": map[string]any{
								"url": fmt.Sprintf("data:%s;base64,%s", mime, b64),
							},
						})
					}
				} else {
					textContent.WriteString(fmt.Sprintf("\n[File: %s]", pt.Filename))
				}
			case *message.ToolResultPart:
				output := a.findToolOutput(all, pt.CallID)
				toolResults = append(toolResults, map[string]any{
					"role":         "tool",
					"content":      output,
					"tool_call_id": pt.CallID,
				})
			}
		}
		if len(imageBlocks) > 0 {
			// Array format required when images are present.
			var content []map[string]any
			if textContent.Len() > 0 {
				content = append(content, map[string]any{
					"type": "text",
					"text": textContent.String(),
				})
			}
			content = append(content, imageBlocks...)
			out = append(out, map[string]any{"role": "user", "content": content})
		} else if textContent.Len() > 0 {
			out = append(out, map[string]any{"role": "user", "content": textContent.String()})
		}
		out = append(out, toolResults...)

	case message.RoleAssistant:
		var content strings.Builder
		var reasoning strings.Builder
		var toolCalls []map[string]any
		for _, p := range m.Parts {
			switch pt := p.(type) {
			case *message.TextPart:
				content.WriteString(pt.Text)
			case *message.ReasoningPart:
				reasoning.WriteString(pt.Text)
			case *message.ToolCallPart:
				args, _ := json.Marshal(pt.State.Input)
				toolCalls = append(toolCalls, map[string]any{
					"id":   pt.CallID,
					"type": "function",
					"function": map[string]any{
						"name":      pt.ToolName,
						"arguments": string(args),
					},
				})
			}
		}
		msg := map[string]any{"role": "assistant"}
		if content.Len() > 0 {
			msg["content"] = content.String()
		} else {
			msg["content"] = nil
		}
		// Opencode's OpenAI-shape proxy sends `reasoning` in SSE and expects
		// `reasoning` back in requests (it translates to Moonshot's native
		// `reasoning_content` internally). Send both for safety.
		msg["reasoning"] = reasoning.String()
		msg["reasoning_content"] = reasoning.String()
		if len(toolCalls) > 0 {
			msg["tool_calls"] = toolCalls
		}
		out = append(out, msg)
	}

	return out
}

func (a *Adapter) readStream(body io.ReadCloser, evCh chan<- provider.Event) {
	defer close(evCh)
	defer body.Close()

	scanner := bufio.NewScanner(body)
	scanner.Split(scanLines)

	var currentTextPart *message.TextPart
	var currentReasoningPart *message.ReasoningPart
	var currentToolCalls map[int]*toolCallAccumulator

	for scanner.Scan() {
		line := scanner.Text()
		if a.dump {
			fmt.Fprintf(os.Stderr, "[RAW SSE] %s\n", line)
		}
		if !strings.HasPrefix(line, "data: ") {
			continue
		}
		data := strings.TrimPrefix(line, "data: ")
		if data == "[DONE]" {
			if currentTextPart != nil {
				evCh <- provider.EventPartEnd{PartID: currentTextPart.PartID()}
				currentTextPart = nil
			}
			if currentReasoningPart != nil {
				evCh <- provider.EventPartEnd{PartID: currentReasoningPart.PartID()}
				currentReasoningPart = nil
			}
			for _, tc := range currentToolCalls {
				tc.finalize()
				evCh <- provider.EventPartEnd{PartID: tc.part.PartID()}
			}
			evCh <- provider.EventMessageEnd{Finish: "stop"}
			return
		}

		var chunk completionChunk
		if err := json.Unmarshal([]byte(data), &chunk); err != nil {
			evCh <- provider.EventError{Err: fmt.Errorf("unmarshal chunk: %w", err)}
			return
		}

		if len(chunk.Choices) == 0 {
			continue
		}
		delta := chunk.Choices[0].Delta

		if delta.Role == "assistant" && currentTextPart == nil && len(delta.ToolCalls) == 0 {
			// Start of assistant message. Text may come later; create part eagerly
			// so reasoning can precede it without losing the slot.
			currentTextPart = message.NewTextPart(ulid.MustNew(ulid.Now(), nil).String(), "", "", "", false)
			evCh <- provider.EventPartStart{Part: currentTextPart}
		}

		if delta.Content != "" && currentTextPart != nil {
			// Transition: reasoning done, content starting. Close reasoning part.
			if currentReasoningPart != nil {
				evCh <- provider.EventPartEnd{PartID: currentReasoningPart.PartID()}
				currentReasoningPart = nil
			}
			evCh <- provider.EventPartDelta{
				PartID: currentTextPart.PartID(),
				Field:  "text",
				Delta:  delta.Content,
			}
		}

		if delta.Reasoning != "" {
			if currentReasoningPart == nil {
				currentReasoningPart = message.NewReasoningPart(ulid.MustNew(ulid.Now(), nil).String(), "", "", "", "")
				evCh <- provider.EventPartStart{Part: currentReasoningPart}
			}
			evCh <- provider.EventPartDelta{
				PartID: currentReasoningPart.PartID(),
				Field:  "text",
				Delta:  delta.Reasoning,
			}
		}

		for _, tcDelta := range delta.ToolCalls {
			// Tool calls follow reasoning; close it if still open.
			if currentReasoningPart != nil {
				evCh <- provider.EventPartEnd{PartID: currentReasoningPart.PartID()}
				currentReasoningPart = nil
			}
			if currentToolCalls == nil {
				currentToolCalls = make(map[int]*toolCallAccumulator)
			}
			acc, ok := currentToolCalls[tcDelta.Index]
			if !ok {
				part := message.NewToolCallPart(
					ulid.MustNew(ulid.Now(), nil).String(),
					"", "",
					"", "",
					message.ToolState{Status: message.ToolPending, Time: message.ToolTime{Start: time.Now().UnixMilli()}},
				)
				acc = &toolCallAccumulator{part: part}
				currentToolCalls[tcDelta.Index] = acc
				evCh <- provider.EventPartStart{Part: part}
			}
			if tcDelta.ID != "" {
				acc.id = tcDelta.ID
			}
			if tcDelta.Type != "" {
				acc.typ = tcDelta.Type
			}
			if tcDelta.Function.Name != "" {
				acc.name = tcDelta.Function.Name
			}
			if tcDelta.Function.Arguments != "" {
				acc.arguments += tcDelta.Function.Arguments
				evCh <- provider.EventPartDelta{
					PartID: acc.part.PartID(),
					Field:  "arguments",
					Delta:  tcDelta.Function.Arguments,
				}
			}
		}

		if chunk.Choices[0].FinishReason != nil {
			finish := a.mapFinishReason(*chunk.Choices[0].FinishReason)
			if currentTextPart != nil {
				evCh <- provider.EventPartEnd{PartID: currentTextPart.PartID()}
				currentTextPart = nil
			}
			if currentReasoningPart != nil {
				evCh <- provider.EventPartEnd{PartID: currentReasoningPart.PartID()}
				currentReasoningPart = nil
			}
			for _, tc := range currentToolCalls {
				tc.finalize()
				evCh <- provider.EventPartEnd{PartID: tc.part.PartID()}
			}
			currentToolCalls = nil
			evCh <- provider.EventMessageEnd{Finish: finish}
			return
		}
	}

	if err := scanner.Err(); err != nil {
		evCh <- provider.EventError{Err: fmt.Errorf("sse stream: %w", err)}
		return
	}

	// Stream ended without [DONE]; emit message_end with whatever state we have.
	if currentTextPart != nil {
		evCh <- provider.EventPartEnd{PartID: currentTextPart.PartID()}
	}
	if currentReasoningPart != nil {
		evCh <- provider.EventPartEnd{PartID: currentReasoningPart.PartID()}
	}
	for _, tc := range currentToolCalls {
		tc.finalize()
		evCh <- provider.EventPartEnd{PartID: tc.part.PartID()}
	}
	evCh <- provider.EventMessageEnd{Finish: "stop"}
}

type toolCallAccumulator struct {
	part      *message.ToolCallPart
	id        string
	typ       string
	name      string
	arguments string
}

func (acc *toolCallAccumulator) finalize() {
	acc.part.ToolName = acc.name
	acc.part.CallID = acc.id
	if acc.arguments != "" {
		var input map[string]any
		if err := json.Unmarshal([]byte(acc.arguments), &input); err == nil {
			acc.part.State.Input = input
		}
	}
}

func (a *Adapter) mapFinishReason(reason string) string {
	switch reason {
	case "stop":
		return "stop"
	case "length":
		return "length"
	case "tool_calls":
		return "tool_use"
	case "content_filter":
		return "error"
	default:
		return "error"
	}
}

type completionChunk struct {
	ID      string `json:"id"`
	Object  string `json:"object"`
	Choices []struct {
		Index int `json:"index"`
		Delta struct {
			Role             string     `json:"role"`
			Content          string     `json:"content"`
			Reasoning        string     `json:"reasoning"`
			ToolCalls        []toolCall `json:"tool_calls"`
		} `json:"delta"`
		FinishReason *string `json:"finish_reason"`
	} `json:"choices"`
}

type toolCall struct {
	Index    int `json:"index"`
	ID       string `json:"id"`
	Type     string `json:"type"`
	Function struct {
		Name      string `json:"name"`
		Arguments string `json:"arguments"`
	} `json:"function"`
}

// scanLines is a bufio.SplitFunc that handles \r\n, \n, and preserves the line ending.
func scanLines(data []byte, atEOF bool) (advance int, token []byte, err error) {
	if atEOF && len(data) == 0 {
		return 0, nil, nil
	}
	if i := bytes.Index(data, []byte("\n")); i >= 0 {
		// We have a full newline-terminated line.
		return i + 1, dropCR(data[0:i]), nil
	}
	// If we're at EOF, we have a final, non-terminated line. Return it.
	if atEOF {
		return len(data), dropCR(data), nil
	}
	// Request more data.
	return 0, nil, nil
}

// dropCR drops a terminal \r from the data.
func dropCR(data []byte) []byte {
	if len(data) > 0 && data[len(data)-1] == '\r' {
		return data[0 : len(data)-1]
	}
	return data
}
