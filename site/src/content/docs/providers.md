---
title: Providers
description: Configure and use different LLM providers with mew.
---

mew supports multiple LLM providers. Each provider has a "shape" that
determines how mew communicates with it.

## Built-in providers

| Provider | Shape | Default model |
|----------|-------|---------------|
| `opencode-zen` | openai | `deepseek-v4-flash` |
| `opencode-go` | openai | - |
| `z-ai` | openai | - |
| `deepseek` | openai | `deepseek-v4-flash` |
| `umans` | anthropic | - |

## Provider shapes

Two shapes are supported:

- **`openai`**: SSE, delta-based streaming. Used by OpenAI-compatible APIs
  (DeepSeek, OpenAI, openai-compatible gateways).
- **`anthropic`**: SSE, content-block events. Used by Anthropic-compatible
  APIs (Claude, Z.AI, Umans).

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

## Router provider

The router wraps any two providers behind the same `Provider` trait with
automatic failover. It picks the cheap model for simple turns and switches
to the capable model when tool calls appear or the conversation exceeds a
turn threshold (default 3).

```toml
[providers.smart]
kind = "router"
small = "deepseek/deepseek-v4-flash"
big = "z-ai/glm-4.5-air"
```

The display model name reflects what the user chose, so a single session
can bounce between models without the user knowing.

## Model catalog

mew includes a built-in model catalog (from models.dev with 24-hour cache)
that provides pricing, context windows, capabilities, and thinking variant
defaults. The catalog is used to:

- Populate the model picker (`Ctrl+P` → "Switch Model")
- Resolve thinking variant names to provider-specific params
- Show pricing in the `/cost` command
- Set context windows for compaction thresholds

## Thinking variants

Some models support configurable thinking/reasoning levels. Available
variants depend on the model:

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

Set with `/thinking <variant>` or `Ctrl+P` → "Thinking Variant".
