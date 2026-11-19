---
title: Providers
description: Configure and use different LLM providers with mew.
---

Each provider has a shape that tells mew which API to call. You can
choose a built-in provider or add your own.

## Built-in providers

| Provider | Shape | Credential environment variable |
|----------|-------|---------------------------------|
| `opencode-zen` | `openai` | `MEW_CRED_OPENCODE_ZEN` |
| `opencode-go` | `openai` | `MEW_CRED_OPENCODE_ZEN` |
| `z-ai` | `openai` | `MEW_CRED_Z_AI` |
| `deepseek` | `openai` | `MEW_CRED_DEEPSEEK` |
| `umans` | `anthropic` | `MEW_CRED_UMANS` |
| `codex` | `responses` | `MEW_CRED_CODEX` for API key authentication |
| `kimi-for-coding` | `openai` | `MEW_CRED_KIMI_FOR_CODING` |
| `alibaba-token-plan` | `openai` | `MEW_CRED_ALIBABA_TOKEN_PLAN` |
| `alibaba-token-plan-cn` | `openai` | `MEW_CRED_ALIBABA_TOKEN_PLAN_CN` |

OpenCode Zen and Go share a credential. The China and international Alibaba
Token Plan endpoints use separate credentials.

For Codex, run `mew auth login codex` to sign in with ChatGPT. Run
`mew auth status` to check your sign-in and `mew auth logout codex` to sign out.

## Provider shapes

| Shape | API |
|-------|-----|
| `openai` | OpenAI-compatible Chat Completions |
| `anthropic` | Anthropic-compatible Messages |
| `responses` | OpenAI Responses |

The shape determines how mew makes requests and parses streaming responses.
See [Adding a Provider](/docs/development/dev-providers/) for adapter details.

## Adding a custom provider

Add a `[providers.<id>]` section to your `config.toml`:

```toml
[providers.my-provider]
shape = "openai"
base_url = "https://api.example.com/v1"
credential_ref = "MY_API_KEY"
```

Set the credential and select the model with its provider prefix.

```sh
export MEW_CRED_MY_API_KEY="sk-..."
mew chat --model my-provider/my-model-id
```

The `credential_ref` is resolved through env vars, keyring, or
`credentials.json`. See [Configuration](/docs/getting-started/configuration/#credentials)
for the resolution order.

## Router provider

A task router assigns models to tiers for subagents and permission
classification. For chat, select a model from a direct provider.

```toml
[providers.router]
shape = "openai"
kind = "router"
micro = "deepseek/deepseek-v4-flash"
deci = "deepseek/deepseek-v4-pro"
```

Use `model: micro` or `model: deci` in a subagent definition to select
that tier. You can also set an optional `nano` model. Without it,
`model: nano` uses the `micro` model. The permission classifier uses
`micro` when a task router is configured. The configured provider for
each tier supplies its own credentials.

## Model catalog

mew’s model catalog provides pricing, context windows, capabilities, and
thinking options. It loads catalog data from models.dev and supplements
it with provider-specific catalogs where available. The catalog is used to:

- Populate the model picker (`Ctrl+Space` and select "Switch Model")
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

Available variants depend on both the provider and the model. In the
TUI, run `/thinking` to see the choices for your current model.

Set a variant with:

```
/thinking high
```

Or open `Ctrl+Space` and select "Thinking Variant". Press `Ctrl+Alt+P` repeatedly
to cycle through the available options for the current model.

To disable thinking:

```
/thinking off
```

When no variant is set, mew uses the catalog default for the model.
