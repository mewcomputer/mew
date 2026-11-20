---
title: Quick Start
description: Configure a provider and send your first prompt in the terminal.
---

## 1. Configure a provider

After [installing Mew](/docs/getting-started/installation/), set credentials
for one of the built-in providers. For example, set the API key from an
OpenCode Zen account in your shell:

```sh
export MEW_CRED_OPENCODE_ZEN="your-api-key"
```

See [Providers](/docs/using-mew/providers/) for other API key options or
[sign in to Codex](/docs/using-mew/providers/#sign-in-to-codex) with ChatGPT.
Use `mew config edit` to add a custom provider.

## 2. Start a chat

```sh
mew chat --provider opencode-zen
```

Use `mew` to start with your last-used provider on later launches.

This opens the TUI. The layout has three areas:

```
┌──────────────────────────────────────────────┐
│  status bar: model · provider · context      │
├──────────────────────────────────┬───────────┤
│                                  │  sidebar  │
│  chat                            │           │
│                                  │  session  │
│  > your messages                 │  todos    │
│  assistant responses             │  changes  │
│  tool call cards                 │           │
│                                  │           │
├──────────────────────────────────┴───────────┤
│  input bar (type your prompt here)           │
└──────────────────────────────────────────────┘
```

- **Status bar** (top): shows the current model, provider, and context
  window usage.
- **Sidebar** (right): the current session at the top, then live activity
  (todos, subagents, background jobs, file changes) as it happens. Press
  `Ctrl+1` to expand the Environment section and see loaded context files.
- **Chat** (center): messages stream in as the model responds. Tool calls
  appear as cards with their input and output.
- **Input bar** (bottom): type your prompt and press `Enter`.

Type a prompt and press Enter. The response streams in as it's generated.

## 3. One-shot mode

For quick prompts without the TUI:

```sh
mew run --provider opencode-zen "explain what this project does"
```

This runs a single turn, prints the response, and exits. Useful for
scripting and quick questions.

For a machine-readable stream, use `mew run --format jsonl "prompt"`.
Each stdout line is a JSON event. `run_started` identifies the session and
model; `text_delta` contains response text; `model_response` reports a model
response and its usage; `tool_start`/`tool_end` and
`nested_tool_start`/`nested_tool_end` track tool calls. The final
`run_finished` event reports status, elapsed milliseconds, call counts, usage
and cost. Events after `run_started` carry elapsed milliseconds. Early command
or configuration errors can occur before the stream starts; check the exit
status and stderr as well. Diagnostic output stays on stderr.

Tool-end events report text output size in bytes when available. The summary
records the largest top-level and nested text outputs separately. A nested output
stays inside Python unless the cell prints it, so its size is not necessarily
the amount sent to the model.

The stream also reports retries, permission decisions, cancelled questions and
goals, and subagent starts and finishes. Noninteractive runs automatically
allow permission prompts.

Usage and call totals cover the top-level agent, including tools called from
Python. They do not include child subagent usage. Usage and cost come from the
provider and may be incomplete. Tool inputs and outputs are not included in
the event stream; the response text and error messages can contain task data.

## 4. Switch models

Inside the TUI, press `Ctrl+Space` to open the command palette, then select
"Switch Model" and choose a model. You can also use `/model provider/model`
with a model from one of your configured providers. The picker lists
available models from those providers and the built-in catalog. See [Providers](/docs/using-mew/providers/) for the full
list.

## 5. Set thinking variant

For models that support configurable reasoning:

```
/thinking high
```

Or use `Ctrl+Space` and select "Thinking Variant". In that picker,
`Ctrl+Alt+P` cycles through available options. See
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
- [Keyboard Shortcuts](/docs/getting-started/keyboard-shortcuts/): terminal keybindings
- [Tips & Tricks](/docs/using-mew/tips-and-tricks/): power-user features and
  workflows
- [Permissions](/docs/using-mew/permissions/): how tool approval works
