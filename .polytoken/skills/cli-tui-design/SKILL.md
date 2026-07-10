---
name: cli-tui-design
description: Design low-friction command-line and terminal-UI tools that behave well for every caller — humans at an interactive terminal, scripts and pipes, and AI agents or harnesses driving them programmatically. Use whenever the user is building or improving a CLI or TUI — adding commands or subcommands, designing flags and arguments, deciding on output format, handling interactive prompts/selectors, wiring exit codes, or debugging friction like a menu dumped into stdout when nothing's there to click, tools split across multiple binaries, output that can't be parsed when piped, or prompts that hang with no human present. Also use for reviewing an existing CLI/TUI's ergonomics.
---

# CLI / TUI Design

Most command-line friction comes from one root cause: the tool assumes a context it doesn't actually have. It renders an interactive selector when nothing is attached to click it. It blocks on a confirmation prompt when no human is present to answer. It prints pretty colored columns that turn to escape-code soup the moment the output is piped. It hides its serving command in a separate binary that only a human browsing `--help` would ever find.

The through-line for a frictionless tool is: **know who's calling you, and fit yourself to them.** Your callers are not all the same, and the same output that delights one corrupts another.

## The three callers

Design for all three from the start. Detect which you're facing and adapt.

1. **A human at an interactive terminal.** Wants affordances: colored output, progress spinners, selectors and prompts, a pager for long output, helpful nudges. Attached to a TTY.
2. **A script or pipe.** Output is being captured, greped, or fed to the next command. Wants plain, stable, parseable text — no colors, no spinners, no cursor movement, no interactive prompts. Not a TTY.
3. **An AI agent or harness.** Increasingly the caller. It's driving your tool programmatically and reading whatever comes back on stdout/stderr. It cannot click a selector, cannot answer a prompt, and will happily hang forever waiting on one — or worse, scrape a half-rendered interactive UI as if it were data. Treat it like the script case, but assume it wants structured output and will get stuck on anything interactive.

The single highest-leverage habit: **detect whether stdout (and stdin) is a TTY, and gate every interactive affordance on it.** When you're not attached to an interactive terminal, don't render the selector — fall back to a flag-driven choice or an error that says which flag to pass. Don't start the spinner — print plain progress lines or nothing. Don't page. Don't prompt — read the value from a flag, env var, or stdin, and if it's genuinely required and absent, fail fast with a message naming exactly what to provide.

## Every interactive path needs a non-interactive twin

This is the rule that would have prevented the "selector dumped into the chat" problem. Any moment your tool would stop and ask a human something, there must be a way to supply that answer up front without the prompt:

- A selector/menu → a `--choice X` flag (and listing the valid choices in `--help` and in the error when it's missing).
- A yes/no confirmation → a `--yes`/`--force` flag, and reading non-interactively should *not* silently assume yes; it should require the flag.
- A "press enter to continue" → gone entirely in non-interactive mode.
- A credential prompt → an env var or stdin, never only a hidden interactive prompt.

If you can list what an interactive session would ask, you can list the flags that pre-answer it. A tool that can only be driven by a human present at the keyboard is a tool no harness can use.

## One coherent entry point

Fragmentation is friction. If your TUI or serving mode lives in a *separate binary* from the main CLI, a caller has to already know it exists to find it — it won't show up in the main tool's `--help`, and an agent has no path to discover it. Prefer one binary with subcommands:

```
mytool build
mytool serve        # the TUI/serving mode, discoverable as a subcommand
mytool serve --headless   # non-interactive twin of the TUI
```

over `mytool` and a stray `mytool-tui` binary. One entry point means one place to discover the whole surface, consistent flag conventions across modes, and a natural home for shared config. Keep subcommand verbs consistent (`get`/`list`/`create`/`delete`, not `get`/`show`/`make`/`rm` scattered by mood). If a TUI genuinely must ship separately, at minimum make the main tool advertise it in `--help`.

## Output belongs to whoever's reading it

Separate the two audiences explicitly rather than hoping one format serves both:

- **Human-facing output** can be pretty — but only when attached to a TTY. Strip color and formatting when piped (respect `NO_COLOR` and the not-a-TTY signal).
- **Machine-facing output**: offer a `--json` or `--format` flag that emits stable, structured data. A harness should never have to regex your human prose to get a value. Once you offer it, keep the shape stable — it's an API now.
- **stdout is for data, stderr is for diagnostics.** Progress, logs, and status go to stderr so they don't corrupt the data a pipe is capturing on stdout. This one split fixes a huge share of "why is my piped output garbled" pain.
- Offer `--quiet` (data only) and `--verbose` (more diagnostics) so callers tune the noise to their context.

## Exit codes and composability

- **Exit 0 on success, non-zero on failure — meaningfully.** Callers (and harnesses especially) branch on the exit code; a tool that returns 0 while failing lies to everything downstream. Use distinct non-zero codes for distinct failure classes where it helps (e.g. 1 general, 2 usage error).
- **Read stdin** when it makes sense, so your tool composes in a pipe rather than demanding a file path.
- **Don't require a TTY to run.** If your tool crashes or hangs when stdin isn't a terminal, no pipeline and no harness can use it.

## No surprising destructive behavior

Anything that deletes, overwrites, or is otherwise hard to undo needs a safety rail — but the rail must not itself become a hang:

- Offer `--dry-run` to show what *would* happen.
- For destructive actions, confirm — but only interactively. Non-interactively, require an explicit `--yes`/`--force` rather than prompting (which would hang) or silently proceeding (which would surprise). Absent the flag and absent a human, fail with a message saying to pass it.
- Provide an undo or a backup where the operation reasonably allows it.

## Errors that tell you what to do next

An error's job is to get the caller unstuck. "Invalid input" is a dead end. State what was wrong, and what to do about it: `no environment selected; pass --env <name> or set MYTOOL_ENV (valid: dev, staging, prod)`. For an agent, that fix-forward message is often the entire difference between recovery and a stuck loop — it can read the remedy and retry. Put errors on stderr, and make them specific.

## Sensible defaults and discoverability

- **`--help` should actually help:** show usage, the common flags, and a real example, at both the top level and per subcommand. It's the map of your tool's whole surface.
- **Good defaults** mean the common case needs few flags; reserve flags for real variation. Optimize the frequent path.
- Support `--version`, and shell completion if the tool is used often.
- Startup should be fast — a tool invoked in a hot loop or a pipeline pays its startup cost every call.

## Quick self-check

Before shipping a command, ask:

1. If I pipe this into `cat`, is the output clean and parseable, or full of escape codes and spinners?
2. If a harness runs this with no TTY, does it complete, or hang on a prompt / render a selector into the void?
3. Can everything an interactive session would ask be supplied by a flag, env var, or stdin instead?
4. Is every mode (including the TUI) discoverable from the one main entry point?
5. Does a failure exit non-zero, print a fix-forward message to stderr, and leave nothing half-destroyed?
