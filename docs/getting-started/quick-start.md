---
title: Quick Start
description: Get up and running with mew in under a minute.
---

## 1. Configure a provider

mew needs at least one LLM provider. The easiest way is to set an API
key as an environment variable:

```sh
export MEW_CRED_OPENCODE_ZEN="sk-..."
```

Or create a `~/.config/mew/config.toml`. See
[Configuration](/docs/getting-started/configuration/) for the full reference.

## 2. Start a chat

```sh
mew
```

This opens the TUI. The layout has three areas:

```
┌──────────────────────────────────────────────┐
│  status bar: model · provider · context      │
├────────────├──────────────────────────────────┤
│            │                                  │
│  sidebar   │  chat                            │
│            │                                  │
│  context   │  > your messages                 │
│  tools     │  assistant responses              │
│  mcp       │  tool call cards                 │
│            │                                  │
├────────────├──────────────────────────────────┤
│  input bar (type your prompt here)           │
└──────────────────────────────────────────────┘
```

- **Status bar** (top): shows the current model, provider, and context
  window usage.
- **Sidebar** (left): toggle sections with `Ctrl+1` (context), `Ctrl+2`
  (tools), `Ctrl+3` (MCP servers). The sidebar shows whenever the terminal
  is wide enough.
- **Chat** (center): messages stream in as the model responds. Tool calls
  appear as cards with their input and output.
- **Input bar** (bottom): type your prompt and press `Enter`.

Type a prompt and press Enter. The response streams in as it's generated.

## 3. One-shot mode

For quick prompts without the TUI:

```sh
mew run "explain what this project does"
```

This runs a single turn, prints the response, and exits. Useful for
scripting and quick questions.

## 4. Switch models

Inside the TUI, press `Ctrl+P` to open the command palette, then select
"Switch Model". Or use the slash command:

```
/model deepseek-v4-flash
```

The model picker shows available models from your configured providers
and the built-in catalog. See [Providers](/docs/using-mew/providers/) for the full
list.

## 5. Set thinking variant

For models that support configurable reasoning:

```
/thinking high
```

Or use `Ctrl+P` and select "Thinking Variant". Press `Ctrl+P` repeatedly
to cycle through available options. See
[Providers](/docs/using-mew/providers/#thinking-variants) for which models support
which variants.

## 6. Cancel a stream

Press `Esc` twice to cancel the current response. The first `Esc` shows
a hint in the status bar ("esc again to stop agent"). The second cancels
immediately.

## 7. Review costs

```
/cost
```

Shows accumulated token counts and estimated cost for the session.

## Where to go next

- [Slash Commands](/docs/using-mew/slash-commands/): full command reference
- [Keyboard Shortcuts](/docs/getting-started/keyboard-shortcuts/): all TUI keybindings
- [Tips & Tricks](/docs/using-mew/tips-and-tricks/): power-user features and
  workflows
- [Permissions](/docs/using-mew/permissions/): how tool approval works
