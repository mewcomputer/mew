---
title: Quick Start
description: Configure a provider and send your first prompt in the terminal.
---

## 1. Configure a provider

After [installing Mew](/docs/getting-started/installation/), set credentials
for a built-in provider. For OpenCode Zen, export your API key in your shell:

```sh
export MEW_CRED_OPENCODE_ZEN="your-api-key"
```

See [Providers](/docs/using-mew/providers/) for other API key options or
[sign in to Codex](/docs/using-mew/providers/#sign-in-to-codex) with ChatGPT.
Run `mew config edit` to add a custom provider.

## 2. Start a chat

```sh
mew chat --provider opencode-zen
```

Run `mew` on later launches to reuse your last provider.

An empty session opens on a centered landing screen with the composer and a
bottom status bar. Once you send a prompt, the terminal switches to the chat
layout:

```text
┌──────────────────────────────────┬───────────┐
│                                  │  sidebar  │
│  chat                            │           │
│                                  │  session  │
│  > your messages                 │  todos    │
│  assistant responses             │  changes  │
│  tool call cards                 │           │
│                                  │           │
├──────────────────────────────────┤  (when    │
│  input bar (type your prompt)    │  >= 112   │
├──────────────────────────────────┤  columns) │
│  status bar: model · cwd · cost  │           │
└──────────────────────────────────┴───────────┘
```

- **Chat** (main pane): displays your prompts, streamed model replies, and
  tool-call cards with inputs and output.
- **Input bar** (below chat): accepts prompts, slash commands, and `@`
  references. Press `Enter` to send.
- **Status bar** (bottom): shows the permission mode (when non-standard),
  provider and model, active goal, thinking variant, persona, working
  directory, git branch, context usage, cost, and attention bell.
- **Sidebar** (right, shown at 112 columns or wider): displays the active
  session, goal, todos, subagents, background jobs, and file changes. Press
  `Ctrl+1` to toggle the Environment section and inspect loaded context files.

## 3. One-shot mode

Run a single turn and print the result to stdout:

```sh
mew run --provider opencode-zen "explain what this project does"
```

For a machine-readable JSONL event stream on stdout, pass `--format jsonl`:

```sh
mew run --format jsonl "explain what this project does"
```

Key JSONL events:

- `run_started`: identifies the session and model.
- `text_delta`: streams assistant response text.
- `model_response`: reports each model response and its token usage.
- `tool_start` / `tool_end` and `nested_tool_start` / `nested_tool_end`: track
  top-level and Python-nested tool calls, including `elapsed_ms` and text output
  `output_bytes`.
- `run_finished`: reports final status, `elapsed_ms`, call counts, top-level
  usage and cost, and the largest top-level and nested output sizes.

Noninteractive runs emit retries, permission decisions, cancelled questions and
goals, and subagent lifecycle events. Every event after `run_started` carries
`elapsed_ms`. Diagnostic output goes to stderr, so check both the exit status
and stderr when a command fails before streaming starts.

Noninteractive runs allow permission prompts automatically, though explicit
`deny` rules in `config.toml` still block matching tool calls. Usage and call
totals include tools invoked from Python, exclude child subagents, and rely on
provider-reported figures that may be incomplete. Nested tool outputs stay
inside Python unless printed, so `output_bytes` can exceed what enters the
model's context. Tool arguments and outputs stay out of the JSONL stream, though
assistant text and error messages can include task data.

## 4. Switch models

Press `Ctrl+Space` in the TUI and select **Switch Model**, or run
`/model provider/model`. The picker lists models from your configured providers
and the built-in catalog. See [Providers](/docs/using-mew/providers/) for the
full list.

## 5. Set thinking variant

For models with configurable reasoning, run:

```text
/thinking high
```

You can also press `Ctrl+Space` and select **Thinking Variant**. Inside that
picker, `Ctrl+Alt+P` cycles through the model's supported variants. See
[Providers](/docs/using-mew/providers/#thinking-variants) for supported models
and options.

## 6. Queue, steer, and cancel

Submitting a prompt while a response is streaming queues it for the next turn.
Press `Ctrl+Up` to inject the oldest queued message as guidance into the
running turn, or press `Up` twice with an empty composer to cancel the active
turn and send the queued message immediately.

Press `Esc` twice to cancel an active turn without sending a queued message. The
first `Esc` displays `"esc again to stop agent"` in the status bar; the second
cancels the stream immediately.

## 7. Attach images

For models with vision support, paste a clipboard image with `Ctrl+V` (or
`Cmd+V`), or reference an image file with `@path/to/image.png`.

## 8. Review costs

```text
/cost
```

Prints accumulated token counts and estimated cost for the current session.

## Where to go next

- [Slash Commands](/docs/using-mew/slash-commands/): full command reference.
- [Keyboard Shortcuts](/docs/getting-started/keyboard-shortcuts/): terminal keybindings.
- [Tips & Tricks](/docs/using-mew/tips-and-tricks/): power-user workflows.
- [Permissions](/docs/using-mew/permissions/): how tool approval works.
