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
| `antigravity` | local `agy` CLI | Existing AGY sign-in |
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

## Use Antigravity

Mew uses the official `agy` executable on `PATH` and its existing local
account. If needed, run `mew auth login antigravity` to sign in through AGY.
The model picker discovers models through that installation.

AGY runs without an OS sandbox by default. Mew restricts AGY's native tools
and routes client tool calls through Mew's permission policy, but a failure in
AGY's own restrictions could expose local files or commands. `mew run` is
non-interactive and automatically approves permission requests; explicit deny
rules still apply. Interactive clients can show permission requests normally.
AGY also starts enabled MCP servers from its own global configuration, including
during model discovery. Those servers run outside Mew's permission system;
disable them in AGY if you do not want them active during Mew sessions.

To request an OS sandbox, add this to `config.toml`:

```toml
[antigravity]
isolated = true
```

The macOS sandbox currently blocks a listener required by AGY. On other
platforms, isolation is unavailable and the provider fails closed.

Select an available model, for example:

```sh
mew run --provider antigravity --model gemini-4-argon-low "Reply with hello."
```

AGY's native subagents and browser tools, attachments, and structured output
are not supported by this bridge. Mew's own tools and subagents remain
available through the MCP bridge.

## Provider shapes

| Shape | Protocol |
|-------|----------|
| `openai` | OpenAI-compatible Chat Completions |
| `anthropic` | Anthropic-compatible Messages |
| `responses` | OpenAI Responses |
| `antigravity` | Local AGY CLI bridge |

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
