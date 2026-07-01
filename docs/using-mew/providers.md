---
title: Providers
description: Configure and use different LLM providers with mew.
---

mew supports multiple LLM providers. Each provider has a "shape" that
determines how mew communicates with it. You can use built-in providers,
add custom ones, or wrap multiple providers behind a router.

## Built-in providers

These are configured out of the box. Set the credential and you're ready.

| Provider | Shape | Default model | Credential env var |
|----------|-------|---------------|-------------------|
| `opencode-zen` | openai | `deepseek-v4-flash` | `MEW_CRED_OPENCODE_ZEN` |
| `opencode-go` | openai | - | `MEW_CRED_OPENCODE_ZEN` |
| `z-ai` | openai | - | `MEW_CRED_Z_AI` |
| `deepseek` | openai | `deepseek-v4-flash` | `MEW_CRED_DEEPSEEK` |
| `umans` | anthropic | - | `MEW_CRED_UMANS` |

`opencode-zen` and `opencode-go` share the same credential. `umans` uses
the Anthropic shape and hits `/v1/messages` with `x-api-key` headers.

## Provider shapes

Two shapes are supported:

- **`openai`**: SSE, delta-based streaming. Used by OpenAI-compatible
  APIs (DeepSeek, OpenAI, openai-compatible gateways). Streams
  `choices[0].delta.content` as text deltas.
- **`anthropic`**: SSE, content-block events. Used by Anthropic-compatible
  APIs (Claude, Z.AI, Umans). Uses named events like `content_block_start`
  and `content_block_delta`. Thinking blocks become `Part::Reasoning`.

The shape determines how mew parses the SSE stream and maps it to
`ProviderEvent` variants. See [Adding a Provider](/docs/development/dev-providers/)
for implementation details.

## Adding a custom provider

Add a `[providers.<id>]` section to your `config.toml`:

```toml
[providers.my-provider]
shape = "openai"
base_url = "https://api.example.com/v1"
credential_ref = "MY_API_KEY"
model = "my-model-id"
```

Then set the credential:

```sh
export MEW_CRED_MY_API_KEY="sk-..."
```

The `credential_ref` is resolved through env vars, keyring, or
`credentials.json`. See [Configuration](/docs/getting-started/configuration/#credentials)
for the resolution order.

## Router provider

The router wraps two providers (small + big) behind the same `Provider`
trait with automatic switching. It picks the cheap model for simple turns
and switches to the capable model when tool calls appear or the
conversation exceeds a turn threshold.

```toml
[providers.smart]
shape = "openai"
kind = "router"
small = "deepseek/deepseek-v4-flash"
big = "z-ai/glm-4.5-air"
credential_ref = "deepseek"
```

### How the router decides

The router starts with the small model. It switches to the big model
when either:

- **Tool calls appear** in the response: the small model started
  generating tool calls, so the router hands off to the big model for
  the rest of the turn.
- **Turn threshold exceeded** (default 3): after 3 turns in one
  agentic loop, the router switches to the big model for robustness.

The `Routed` wrapper preserves the display model name so the TUI status
line shows what you chose, even though the actual model may differ per
turn. A single session can bounce between models without you knowing.

## Model catalog

mew includes a built-in model catalog (from models.dev with 24-hour
cache) that provides pricing, context windows, capabilities, and thinking
variant defaults. The catalog is used to:

- Populate the model picker (`Ctrl+P` and select "Switch Model")
- Resolve thinking variant names to provider-specific params
- Show pricing in the `/cost` command
- Set context windows for compaction thresholds

You can override or extend the catalog with `[[models]]` entries in
`config.toml`. See [Configuration](/docs/getting-started/configuration/#custom-models)
for the format.

## Thinking variants

Some models support configurable thinking/reasoning levels. When you
set a thinking variant, mew translates it into the provider-specific
parameters the model expects.

Available variants depend on the model:

| Model | Variants |
|-------|----------|
| DeepSeek V4 | `high`, `max` |
| GLM 5.2 | `high`, `max` |
| GLM 5/5.1 | `thinking` |
| GPT-5 family | `minimal`, `low`, `medium`, `high` |
| Claude Opus 4.7+ / Fable | `low`, `medium`, `high`, `xhigh`, `max` |
| Claude Sonnet 4.6 | `low`, `medium`, `high`, `max` |
| MiMo v2.5 | `thinking` |
| MiniMax M3 | `thinking`, `none` |
| Grok-3-mini | `low`, `high` |

Note that if you use a supported provider that provides thinking variants (e.g. umans), we will use those.

Set a variant with:

```
/thinking high
```

Or open `Ctrl+P` and select "Thinking Variant". Press `Ctrl+P` repeatedly
to cycle through the available options for the current model.

To disable thinking:

```
/thinking off
```

When no variant is set, mew uses the catalog default for the model.
