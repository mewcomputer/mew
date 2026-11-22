---
title: Providers
description: Configure and use LLM providers with Mew.
---

Each provider defines an adapter shape that tells Mew which API protocol to
speak. You can use the built-in providers or configure custom endpoints.

## Built-in providers

| Provider | Default shape | Credential environment variable |
|----------|---------------|---------------------------------|
| `opencode-zen` | `openai` | `MEW_CRED_OPENCODE_ZEN` |
| `opencode-go` | `openai` (`anthropic` for `minimax-*` models) | `MEW_CRED_OPENCODE_ZEN` |
| `z-ai` | `openai` | `MEW_CRED_Z_AI` |
| `deepseek` | `openai` (`responses` for `deepseek-v4-flash`) | `MEW_CRED_DEEPSEEK` |
| `umans` | `anthropic` | `MEW_CRED_UMANS` |
| `codex` | `responses` | `MEW_CRED_CODEX` (API key) or OAuth |
| `kimi-for-coding` | `openai` | `MEW_CRED_KIMI_FOR_CODING` |
| `alibaba-token-plan` | `openai` | `MEW_CRED_ALIBABA_TOKEN_PLAN` |
| `alibaba-token-plan-cn` | `openai` | `MEW_CRED_ALIBABA_TOKEN_PLAN_CN` |

OpenCode Zen and Go share `MEW_CRED_OPENCODE_ZEN`. The international and China
Alibaba Token Plan providers use separate credentials.

## Sign in to Codex

Run `mew auth login codex` and complete the ChatGPT sign-in in your browser.
The browser opens `auth.openai.com` and redirects to Mew's local callback. On
headless or SSH hosts, pass `--headless` to use the device-code flow:

```sh
mew auth login --headless codex
```

The CLI prints `Login successful for Codex (ChatGPT).` Run `mew auth status` and
confirm the `codex: OAuth` entry.

Select a Codex model explicitly on your first prompt, because Mew's default model
may belong to another provider:

```sh
mew run --provider codex --model gpt-5.6-sol "Reply with hello."
```

Run `mew auth logout codex` to delete Mew's saved OAuth session.

## Provider shapes

| Shape | Protocol |
|-------|----------|
| `openai` | OpenAI-compatible Chat Completions |
| `anthropic` | Anthropic-compatible Messages |
| `responses` | OpenAI Responses |

The adapter shape selects how Mew formats requests and parses streaming
responses. See [Adding a Provider](/docs/development/dev-providers/) for
adapter internals.

## Adding a custom provider

Add a `[providers.<id>]` block to `config.toml`:

```toml
[providers.my-provider]
shape = "openai"
base_url = "https://api.example.com/v1"
credential_ref = "MY_API_KEY"
```

Set `disable_hashline = true` on a `[providers.<id>]` block to omit
`edit_hashline` from that provider's tools. Export the credential and launch with
a provider-prefixed model:

```sh
export MEW_CRED_MY_API_KEY="sk-..."
mew chat --model my-provider/my-model-id
```

Mew resolves `credential_ref` via environment variables, the system keyring, or
`credentials.json`. See
[Configuration](/docs/getting-started/configuration/#credentials) for the full
resolution order.

## Router provider

A task router maps capability tiers (`nano`, `micro`, `deci`) to concrete
models for subagents and the permission classifier. Choose a direct provider
for interactive chat.

```toml
[providers.router]
shape = "openai"
kind = "router"
micro = "deepseek/deepseek-v4-flash"
deci = "deepseek/deepseek-v4-pro"
```

Set `model: nano`, `model: micro`, or `model: deci` in a subagent definition to
route through that tier. If `nano` is omitted, `model: nano` falls back to
`micro`. The permission classifier uses `micro` when a router provider is
configured. Each tier resolves credentials from its target provider.

## Model catalog

Mew's catalog supplies pricing, context windows, model capabilities, and
thinking options using models.dev data and provider-specific catalogs:

- Populates the model picker (`Ctrl+Space`, then **Switch Model**).
- Resolves thinking variant names to provider request parameters.
- Computes session cost estimates for `/cost`.
- Sets context window limits for automatic compaction.

Override or add entries with `[[models]]` blocks in `config.toml`. See
[Configuration](/docs/getting-started/configuration/#custom-models) for the
schema.

## Thinking variants

Models with configurable reasoning accept named thinking variants or token
budgets, which Mew translates into the provider's native request parameters.
Available choices depend on both provider and model; run `/thinking` in the TUI
to list options for your active model.

Set a variant in the TUI:

```text
/thinking high
```

Common options include `low`, `medium`, `high`, `xhigh`, `max`, `thinking`,
`off`, `none`, and `budget:<n>` (for models that declare a token budget range).
You can also press `Ctrl+Space` and select **Thinking Variant**; inside that
picker, press `Ctrl+Alt+P` to cycle through supported options.

Disable reasoning for supported models with:

```text
/thinking off
```

When no variant is set, Mew selects the first supported option among `high`,
`thinking`, `xhigh`, or the first variant other than `none` or `off`.
