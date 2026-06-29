---
title: Adding a Provider
description: How to implement a new LLM provider for mew.
---

mew supports two provider shapes: `openai` (SSE, delta-based) and
`anthropic` (SSE, content-block events). To add a new provider, implement
the `Provider` trait.

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
}
```

- `model`: the model ID (provider-specific, e.g. `"deepseek-v4-flash"`)
- `messages`: full conversation history
- `tools`: tool definitions (name, description, JSON schema)
- `system`: system prompt (already merged with persona body if active)
- `reasoning`: thinking variant config (provider-specific params)
- `params`: temperature, top_p, max_tokens, tool_choice

## ProviderEvent variants

The stream yields these events in order:

```
PartStart { part: Part }
  ↓
PartDelta { part_id, field, delta }   (0 or more)
  ↓
PartEnd { part_id }
  ↓
(repeat for each part in the response)
  ↓
MessageEnd { finish, usage, cost }
```

| Event | Fields | Purpose |
|-------|--------|---------|
| `PartStart` | `part: Part` (Text, Reasoning, or ToolCall) | New part begins |
| `PartDelta` | `part_id`, `field` ("text" or "reasoning"), `delta` | Incremental content |
| `PartEnd` | `part_id` | Part complete |
| `MessageEnd` | `finish: Finish` (Stop, ToolUse, Length, Error), `usage: Tokens`, `cost: f64` | Turn complete |

## OpenAI adapter (`mew-provider-openai`)

The OpenAI adapter handles OpenAI-compatible APIs (DeepSeek, OpenAI,
openai-compatible gateways). Key implementation details:

- Uses `eventsource-stream` to parse SSE from the `/chat/completions` endpoint.
- Maps `choices[0].delta.content` to `PartDelta { field: "text" }`.
- Tool calls arrive fragmented across multiple deltas. A `ToolCallAccumulator`
  buffers partial tool calls by index and emits `PartStart` / `PartEnd` only
  when a tool call is complete.
- `finish_reason` maps to `Finish::Stop` (`"stop"`), `Finish::ToolUse`
  (`"tool_calls"`), or `Finish::Length` (`"length"`).
- Retry logic: exponential backoff on 429/500/502/503 with `RetryPolicy`.

## Anthropic adapter (`mew-provider-anthropic`)

The Anthropic adapter handles Anthropic-compatible APIs (Claude, Z.AI,
Umans). Key differences from OpenAI:

- SSE events have named types: `content_block_start`, `content_block_delta`,
  `content_block_stop`, `message_delta`, `message_stop`.
- Content blocks are typed: `text`, `thinking`, `tool_use`. Each maps to
  a `Part` variant.
- `content_block_delta` events carry `delta.text` or `delta.thinking` or
  `delta.partial_json` (for tool input).
- `message_delta` carries the stop reason and usage.
- Thinking blocks become `Part::Reasoning`. Tool use becomes
  `Part::ToolCall`.

## The router (`mew-provider-router`)

The router wraps two providers (small + big) behind the same `Provider`
trait:

- **Selection logic**: starts with the small (cheap) model. Switches to
  the big (capable) model when:
  - Tool calls appear in the response, OR
  - The conversation exceeds a turn threshold (default 3)
- **Routed wrapper**: preserves the display model name so the TUI status
  line shows what the user chose, even though the actual model may differ
  per turn.
- **Failover**: if the small model errors, the router can fall back to big.

A single session can bounce between models without the caller knowing.

## The fake provider (`mew-provider-fake`)

For tests. `FakeProvider::new(script)` takes a `Vec<ProviderEvent>` and
replays it with a 10ms delay between events:

```rust
async fn stream(&self, _req: Request) -> Result<EventStream, ProviderError> {
    let script = self.script.clone();
    let stream = futures::stream::unfold(script.into_iter(), |mut iter| async move {
        if let Some(event) = iter.next() {
            sleep(Duration::from_millis(10)).await;
            Some((event, iter))
        } else {
            None
        }
    });
    Ok(Box::pin(stream))
}
```

`text_response("hello")` produces a 4-event script: PartStart, PartDelta,
PartEnd, MessageEnd. `tool_call(name, id, input)` produces a tool-call
script ending with `Finish::ToolUse`.

## Adding a new provider

1. Create a `mew-provider-<name>` crate (or add to an existing adapter).
2. Implement `Provider`. Use the OpenAI or Anthropic adapter as reference.
3. Register in `build_provider` (`main.rs`):

```rust
"my-shape" => {
    let adapter = MyAdapter::new(provider_id, base_url, model, credential);
    Ok(Arc::new(adapter))
}
```

4. Add to config defaults in `mew-config/src/lib.rs` if it should be
   available out of the box, or let users configure it in `config.toml`.
5. Add catalog entries if the provider has models in models.dev. The
   catalog provides pricing, context windows, and thinking variant defaults.
6. Write tests using `FakeProvider` as the baseline and your adapter for
   integration tests.
