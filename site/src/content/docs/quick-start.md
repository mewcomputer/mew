---
title: Quick Start
description: Get up and running with mew in under a minute.
---

## 1. Configure a provider

mew needs at least one LLM provider. The easiest way is to set an API key
as an environment variable:

```sh
export MEW_CRED_OPENCODE_ZEN="sk-..."
```

Or create a `~/.config/mew/config.toml`. See [Configuration](/docs/configuration/).

## 2. Start a chat

```sh
mew
```

This opens the TUI. Type a prompt and press Enter.

## 3. One-shot mode

For quick prompts without the TUI:

```sh
mew run "explain what this project does"
```

## 4. Switch models

Inside the TUI, press `Ctrl+P` to open the command palette, then select
"Switch Model". Or use the slash command:

```
/model deepseek-v4-flash
```

## 5. Set thinking variant

For models that support configurable reasoning:

```
/thinking high
```

Or use `Ctrl+P` → "Thinking Variant" and cycle with `Ctrl+P`.

## 6. Cancel a stream

Press `Esc` twice to cancel the current response. The first `Esc` shows a
hint in the status bar; the second cancels immediately.

## Next steps

- [Slash Commands](/docs/slash-commands/): full command reference
- [Keyboard Shortcuts](/docs/keyboard-shortcuts/): all TUI keybindings
