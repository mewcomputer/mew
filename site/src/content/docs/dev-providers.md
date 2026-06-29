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

`Request` carries the messages, tools, system prompt, reasoning config,
and chat params (temperature, max_tokens, tool_choice). `EventStream` is
a pinned boxed `Stream<Item = ProviderEvent>`.

## ProviderEvent variants

The stream yields these events:

- `PartStart { part: Part }` — a new text/reasoning/tool-call part begins
- `PartDelta { part_id, field, delta }` — incremental content for a part
- `PartEnd { part_id }` — part complete
- `MessageEnd { finish, usage, cost }` — turn complete
- `Error { message }` — stream error

## Steps

1. **Create a new crate** `mew-provider-<name>` (or add to an existing one).

2. **Implement `Provider`**. For SSE-based providers, parse the stream into
   `ProviderEvent`s. Use `mew-provider-openai` or `mew-provider-anthropic`
   as a reference.

3. **Register in `build_provider`** (`main.rs`). Add a match arm mapping
   the `shape` string to your adapter:

```rust
"my-shape" => {
    let adapter = MyAdapter::new(base_url, credential, model_id);
    Ok(Arc::new(adapter))
}
```

4. **Add the provider to config defaults** (`mew-config/src/lib.rs`) if it
   should be available out of the box. Or let users configure it manually
   in their `config.toml`.

5. **Add catalog entries** if your provider has models in the models.dev
   catalog. The catalog provides pricing, context windows, and thinking
   variant defaults.

## The router

`mew-provider-router` wraps any two providers behind the same `Provider`
trait with automatic failover. The `Router` picks the cheap model for
simple turns and switches to the capable model when tool calls appear or
the conversation exceeds a turn threshold (default 3). The `Routed`
wrapper preserves the display model name so the TUI status line reflects
what the user chose.

A single session can bounce between models without the user or caller
knowing.

## Fake provider (for tests)

`mew-provider-fake` implements `Provider` with a scripted event sequence.
Use `FakeProvider::text_response("hello")` for simple text responses or
`FakeProvider::tool_call(...)` for tool-call scenarios. Events are emitted
with a 10ms delay between each so cancellation tests can catch mid-stream
state.
