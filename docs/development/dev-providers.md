---
title: Adding a Provider
description: How to implement a new LLM provider for Mew.
---

Mew routes `openai` providers through Chat Completions, `anthropic` through
Messages, and `responses` through the OpenAI Responses API. Implement the
`Provider` trait to add a new adapter.

## The Provider trait

```rust
#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    async fn stream(&self, req: Request) -> Result<EventStream, ProviderError>;
    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        Ok(Vec::new())
    }
}
```

`EventStream` is a pinned boxed async stream:

```rust
pub type EventStream = std::pin::Pin<Box<dyn Stream<Item = ProviderEvent> + Send>>;
```

## The Request struct

```rust
pub struct Request {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDef>,
    pub system: String,
    pub reasoning: Option<ReasoningConfig>,
    pub params: Option<ChatParams>,
    pub headers: http::HeaderMap,
    pub supports_vision: bool,
}
```

- `model`: provider-specific model ID (for example, `"deepseek-v4-flash"`).
- `messages`: conversation history.
- `tools`: tool definitions (name, description, JSON Schema).
- `system`: assembled system prompt (including active persona and context files).
- `reasoning`: provider-specific parameters from the active thinking variant.
- `params`: `temperature`, `top_p`, `max_tokens`, and `tool_choice`.
- `headers`: HTTP headers prepared by the host.
- `supports_vision`: whether the selected model accepts image input.

## ProviderEvent variants

Streams yield events in this sequence:

```text
PartStart { part: Part }
  |
  v
PartDelta { part_id, field, delta }   (0 or more)
  |
  v
PartEnd { part_id }
  |
  v
(repeat for each part)
  |
  v
MessageEnd { finish, usage, cost }
```

| Event | Fields | Purpose |
|-------|--------|---------|
| `PartStart` | `part: Part` (`Text`, `Reasoning`, or `ToolCall`) | Starts a new response part |
| `PartDelta` | `part_id`, `field` (`"text"` or `"reasoning"`), `delta` | Streams incremental content |
| `PartEnd` | `part_id` | Closes a part |
| `MessageEnd` | `finish: Finish`, `usage: Tokens`, `cost: f64` | Completes the provider response |
| `RetryWait` | `attempt`, `max_attempts`, `delay_secs`, `reason` | Signals retry backoff |
| `Error` | `MessageError` | Reports a stream or provider error |

## Built-in adapters

- **`mew-provider-openai`**: parses `/chat/completions` SSE via
  `eventsource-stream`, maps `choices[0].delta.content` to text deltas, buffers
  partial tool arguments in `ToolCallAccumulator` until `PartEnd`, maps
  `finish_reason` (`"stop"` -> `Stop`, `"tool_calls"` -> `ToolUse`, `"length"`
  -> `Length`, `"content_filter"` -> `Error`), and retries 429/500/502/503 with
  `RetryPolicy`.
- **`mew-provider-anthropic`**: consumes named SSE events
  (`content_block_start`, `content_block_delta`, `content_block_stop`,
  `message_delta`, `message_stop`) and typed blocks (`text` -> text, `thinking`
  -> `Part::Reasoning`, `tool_use` -> `Part::ToolCall` with `partial_json`).
- **`mew-provider-responses`**: sends OpenAI Responses API requests using API
  keys for compatible endpoints or Codex OAuth for `codex`.
- **`mew-provider-router`**: `Router` selects `deci` when message history
  contains a tool result, `nano` for the initial turn if configured, and `micro`
  otherwise; `Routed` preserves display labels.
- **`mew-provider-fake`**: `FakeProvider::new(script)` replays `ProviderEvent`s
  at 10ms intervals (`text_response("hello")` emits `PartStart`, four-char
  `PartDelta` chunks, `PartEnd`, and `MessageEnd`; `tool_call` emits a
  `Finish::ToolUse` script).

## Adding a new adapter

1. Create a `mew-provider-<name>` crate implementing `Provider`.
2. Wire the new shape in `build_direct_provider` (called by `build_provider`) in
   `crates/mew/src/setup/providers.rs`, along with any thinking-variant mapping.
3. Add built-in provider defaults in `crates/mew-config/src/lib.rs` (or let
   users configure `[providers.<id>]` in `config.toml`), and update `mew-catalog`
   if the provider maps to models.dev entries.
4. Add fixture tests using `FakeProvider` and adapter unit tests.
