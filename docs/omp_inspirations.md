# What mew can learn from omp (oh-my-pi)

## Executive summary

`omp` (oh-my-pi, `omp.sh`) is a terminal-first coding agent forked from Mario Zechner's Pi. It is deliberately batteries-included: native Rust tooling, LSP/DAP integration, a patched `edit` format, subagents with typed outputs, streaming rule injection, hindsight memory, collaboration relays, and a long list of small workflow tools. It is not a direct competitor in architecture — omp is TypeScript + a Rust N-API addon; mew is Rust end-to-end with a WebSocket daemon + web frontend — but many of its ideas transfer cleanly.

This doc catalogs the most interesting omp features, compares each to mew's current state, and argues for or against adoption. The goal is to feed roadmap decisions, not to prescribe an implementation order.

Three big themes keep showing up:

1. **Stop shelling out on the hot path.** omp embeds grep, glob, bash, search, AST, and highlight as in-process Rust libraries. mew still forks `rg`, `grep`, and `bash` for core tools. Moving these in-process would cut latency, remove binary-availability failures, and make cancellation/secret handling more reliable.
2. **Make the edit format model-friendly.** omp's `hashline` edit format and `ast_edit`/`ast_grep` tools are tuned so models spend fewer tokens and land edits on the first attempt. mew's `edit` tool still uses exact-string replacement and fails on ambiguity or stale files.
3. **Add durable memory and guardrails.** omp has hindsight memory (`retain`/`recall`/`reflect`) and time-traveling stream rules that inject reminders only when the model drifts. mew has session persistence but no cross-session memory or rule-injection runtime.

Below is the full catalog, followed by a prioritized shortlist.

---

## 1. Tool harness & edit formats

### 1.1 Hashline edits (`edit` tool)

**What omp does.** Instead of asking the model to repeat the exact text it wants to replace, omp's `edit` tool uses a "hashline" format: anchors edits by content hashes of the surrounding lines. The model points at hashed anchors and supplies replacement lines. Stale anchors are rejected before any disk write, and whitespace battles disappear because the model no longer retypes the lines.

**omp's claimed impact.** Grok 4 Fast spends ~61% fewer output tokens on the same edits; first-attempt pass rates rise sharply because the retry loop on `old_string not found` goes away.

**mew's current state.** mew's `edit` tool (`crates/mew-tools/src/tools/edit.rs`) uses exact `old_string`/`new_string` replacement. It correctly detects missing/ambiguous matches and returns helpful errors, but it is still fragile on stale files and forces the model to emit large repeated blocks.

**Pro adoption.**
- Fewer tokens per edit, cheaper and faster.
- Rejects stale anchors before corrupting files.
- Eliminates the most common failure mode in long sessions: "the file changed since the last read."

**Anti adoption.**
- Requires designing, documenting, and testing a new patch language (`hashline`).
- Model ecosystems are heavily trained on `str_replace`-style formats; switching formats may hurt models that have not seen hashline.
- mew could get 80% of the benefit with a simpler improvement (multi-line `old_string` + fuzzy anchoring) without inventing a new DSL.

**Verdict.** High value, medium effort. Worth prototyping, but consider a less exotic intermediate first.

### 1.2 `ast_edit` and `ast_grep` tools

**What omp does.** `ast_edit` performs structural rewrites via `ast-grep` and previews them before applying. `ast_grep` runs structural queries over 50+ tree-sitter grammars. Both return a "proposed" card; the agent calls `resolve` to apply or discard. The actual disk move happens atomically.

**mew's current state.** mew has no AST-level tools. Edits are purely textual.

**Pro adoption.**
- Refactors (rename symbol, wrap in try/catch, inline variable) become one-shot operations.
- Preview/resolve flow is a safer UX for mutating tools.
- Fits mew's Rust stack well: `ast-grep-core` and `tree-sitter` crates are available.

**Anti adoption.**
- Adds a large grammar dependency tree.
- Preview/resolve needs UI work in both TUI and web clients.
- Many everyday edits are simple string replacements; AST tools are overkill for those.

**Verdict.** Medium value for a general-purpose agent, high value for refactoring-heavy workflows. Large effort.

### 1.3 Summarizing `read`

**What omp does.** `read` returns summarized snippets with elision controls rather than dumping whole files. It uses tree-sitter structural summaries for source files.

**mew's current state.** mew's `read` tool returns the full file text up to a 10 MB cap, with optional `offset`/`limit`. No summarization.

**Pro adoption.**
- Keeps context windows smaller.
- Models rarely need every import, comment, or blank line.
- Can be done gradually: start with simple heuristics (drop import blocks, collapse long functions) before tree-sitter summaries.

**Anti adoption.**
- Over-summarization can hide the exact line the model needs to edit.
- Requires careful prompt/tool design so the model knows when to ask for the full file.

**Verdict.** High value, small-to-medium effort if done heuristically first.

### 1.4 Preview / resolve flow

**What omp does.** Mutating tools like `ast_edit` stage a proposed change. The TUI shows a "proposed" card. The agent must call `resolve(reason)` to apply or discard. The final "Accept" card confirms the disk move.

**mew's current state.** mew applies `write`/`edit` immediately after permission approval. There is no staging/review step inside the tool surface.

**Pro adoption.**
- Safer for large or structural changes.
- Lets the model plan a batch of edits and apply them atomically.
- Natural fit for AST edits and codemods.

**Anti adoption.**
- Adds a turn of latency for every mutating tool call.
- Requires the model to learn the `resolve` tool.
- For small edits, immediate apply is strictly faster.

**Verdict.** Medium-high value for specific tools (`ast_edit`, bulk renames), probably not for simple `edit`.

---

## 2. Code intelligence

### 2.1 LSP integration

**What omp does.** `lsp` tool exposes diagnostics, go-to-definition, references, symbols, renames, code actions, and raw LSP requests. It is wired into `write` so renames flow through `workspace/willRenameFiles`, updating re-exports and barrel files before the file moves.

**mew's current state.** mew has no LSP integration. The agent reasons about code from raw text only.

**Pro adoption.**
- Makes the agent as code-aware as an IDE.
- Refactors become reliable instead of text-guessing.
- `workspace/willRenameFiles` integration prevents broken imports after file moves.

**Anti adoption.**
- Heavy dependency: needs tower-lsp or similar, per-language server management, and robust server lifecycle handling.
- Language servers are memory-hungry and can crash or hang.
- Per your note, this is likely too much right now.

**Verdict.** Large effort, large payoff, but probably a later-phase project.

### 2.2 DAP / debugger integration

**What omp does.** `debug` tool drives a DAP session: breakpoints, stepping, threads, stack, variables. Supports lldb-dap, dlv, debugpy. The agent can attach to a segfaulting C binary or a hung Go service and inspect state.

**mew's current state.** No debugger integration.

**Pro adoption.**
- Unique capability among terminal agents.
- Useful for systems/codebase debugging.

**Anti adoption.**
- Very heavy dependency surface (debug adapters, per-language setup).
- Fragile across languages and platforms.
- Far outside mew's current scope.

**Verdict.** Low priority unless mew specifically targets systems/debugging workflows.

---

## 3. Runtime & shell

### 3.1 In-process bash (`brush-shell`)

**What omp does.** omp embeds a vendored bash (`brush-shell`) with persistent sessions, timeout/abort, and custom builtins. No fork-exec for every shell call; sessions survive across tool invocations.

**mew's current state.** mew's `bash` tool (`crates/mew-tools/src/tools/bash.rs`) spawns `/bin/bash -c` per call. It handles cancellation by `kill`ing the child PID and returns partial output on timeout, but each call is a fresh process.

**Pro adoption.**
- Faster shell calls; no fork/exec overhead.
- Persistent sessions mean `cd`, `export`, background jobs, and shell state survive between calls.
- Better cancellation and secret redaction control.
- Cross-platform: works on Windows without WSL.

**Anti adoption.**
- Embedding a full POSIX shell is a large dependency (`brush-core` is ~3,700 LoC in omp).
- mew already has separate `ShellBackground`/`ShellMonitor`/`Job*` tools that cover some persistent-job use cases.
- Full bash compatibility is a long tail of edge cases.

**Verdict.** High value, large effort. A lighter first step is to improve mew's existing bash tool with a persistent shell session (e.g., keep one `bash` child alive and pipe commands to it).

### 3.2 In-process grep / glob / find

**What omp does.** ripgrep, glob, and find are linked into the process via native Rust crates (`grep-regex`, `grep-searcher`, `ignore`, `globset`). omp claims this avoids fork-exec round-trips and binary-availability failures.

**mew's current state.** mew's `grep` tool shells out to `rg` with a `grep` fallback. `glob` uses `ignore`/`globset` already in-process (`crates/mew-tools/src/tools/glob.rs`). `bash` shells out.

**Pro adoption.**
- More reliable: no dependency on `rg` being installed.
- Better cancellation integration.
- Easier secret redaction and workspace-root enforcement.

**Anti adoption.**
- mew's `glob` is already in-process; only `grep` needs porting.
- The `grep` crate API is lower-level than `rg`'s CLI; reproducing output format exactly matters for model compatibility.

**Verdict.** Medium-high value, small-to-medium effort for `grep`. A clear quick win.

### 3.3 Persistent Python / JavaScript cells (`eval`)

**What omp does.** `eval` runs persistent Python and Bun worker cells with a shared prelude. Either kernel can call back into the agent's tools (`read`, `search`, etc.) over a loopback bridge.

**mew's current state.** mew has no code-evaluation tool. The agent can run Python/JS via `bash`, but each invocation starts fresh and cannot call back into mew tools.

**Pro adoption.**
- Enables data analysis, plotting, and scripted workflows in one session.
- Tool re-entry from inside the kernel is powerful (e.g., load CSV, analyze, write result).

**Anti adoption.**
- Requires embedding Python and JS runtimes or managing child kernels with a bridge.
- Large security and sandboxing surface.
- Adds significant complexity for a feature many users may not need.

**Verdict.** Medium value, large effort. Consider after core tool harness improvements.

### 3.4 Native PTY for interactive prompts

**What omp does.** Native PTY allocation for `sudo`, `ssh` interactive prompts, etc., via `portable-pty`.

**mew's current state.** mew's bash tool uses piped stdin/stdout. Interactive prompts will hang or fail.

**Pro adoption.**
- Makes `bash` useful for commands that require password/confirmation prompts.
- Standard tool (`portable-pty`) exists for Rust.

**Anti adoption.**
- Permission/safety story gets more complex with PTYs (the agent might approve a hidden prompt).
- Most CI/automated workflows avoid interactive commands.

**Verdict.** Small-to-medium value, small effort. Worth adding to `bash` or as a separate `bash_pty` tool.

---

## 4. Subagents & coordination

### 4.1 First-class `task` fan-out with typed results

**What omp does.** `task` spawns subagents in parallel, optionally in isolated worktrees. The final yield is a schema-validated object the parent reads directly. No prose parsing.

**mew's current state.** mew has `subagent_start` and `subagent_wait` tools plus a `Subagent` schema tool. Subagent sessions are persisted under the parent's session folder. Results appear as `SubagentOutcome` events, but they are not strongly typed JSON schemas.

**Pro adoption.**
- Parent agents can delegate confidently when subagents return structured data.
- Reduces merge conflicts and orphaned edits from sibling subagents.

**Anti adoption.**
- Requires JSON-schema plumbing through the subagent runner.
- mew's existing subagent infrastructure is already close; the gap is mostly tooling and conventions.

**Verdict.** High value, medium effort. A natural evolution of mew's existing subagents.

### 4.2 Inter-agent messaging (`irc`)

**What omp does.** `irc` allows short prose messages between live agents in the same process.

**mew's current state.** No equivalent. Subagents are independent.

**Pro adoption.**
- Useful for coordinating parallel subagents.
- Light feature.

**Anti adoption.**
- Easy to implement but easy to misuse; models may spam messages.
- Value depends on having many parallel subagents, which most sessions do not.

**Verdict.** Low-to-medium value, small effort.

### 4.3 Advisor / reviewer model

**What omp does.** A second model in an "advisor" role reads every main-agent turn and injects notes — concerns, blockers, or quiet asides — inline. It runs on its own context and model.

**mew's current state.** No equivalent. mew has a single provider per session plus the provider-router for cheap/capable switching.

**Pro adoption.**
- Catches mistakes the doer model rushes past.
- Can be implemented as a background subagent or sidecar.

**Anti adoption.**
- Doubles token cost per turn.
- Adds UI complexity (rendering advisor notes alongside main output).
- Advisor noise can be high.

**Verdict.** Medium value, medium effort. Could be prototyped as a subagent that reviews the last turn.

---

## 5. Memory & context

### 5.1 Hindsight memory (`retain` / `recall` / `reflect`)

**What omp does.** The agent can `retain` durable facts into a per-project Hindsight bank, `recall` raw memories, and `reflect` to synthesize answers. Each session is compressed into a mental model loaded on the first turn of the next session.

**mew's current state.** mew persists full sessions to JSONL (`crates/mew-session`) and can resume them, but there is no curated memory bank or summarization of past sessions.

**Pro adoption.**
- Cross-session continuity: the agent remembers codebase conventions without re-reading everything.
- Project-scoped by default, so learnings stay with the repo.

**Anti adoption.**
- Memory quality is hard: stale facts, hallucinated conventions, and overfitting to old code.
- Requires a new storage and retrieval subsystem.

**Verdict.** High value, large effort. Start small: automatic session summary on close, optionally loaded as context on resume.

### 5.2 `checkpoint` / `rewind`

**What omp does.** `checkpoint` marks conversation state for later collapse-and-report. `rewind` prunes exploratory context and keeps a concise report.

**mew's current state.** mew supports `/compact` slash command and reasoning truncation, but no explicit checkpoint/rewind tools.

**Pro adoption.**
- Helps manage long context windows.
- Gives the agent a way to abandon dead ends cleanly.

**Anti adoption.**
- `/compact` already covers some of this.
- Requires model training/coercion to use checkpoints correctly.

**Verdict.** Medium value, small-to-medium effort.

---

## 6. Rules & guardrails

### 6.1 Time-traveling stream rules (TTSR)

**What omp does.** User rules sit dormant until a regex match in the streaming output indicates the model is going off-script. The stream aborts mid-token, the rule is injected as a system reminder, and the request retries from the same point. Injections survive compaction.

**mew's current state.** mew has AGENTS.md / CLAUDE.md context loading and permission rules, but no runtime rule injection triggered by model output.

**Pro adoption.**
- Course-correction without paying context tax on every turn.
- Rules fire only when needed.

**Anti adoption.**
- Complex streaming implementation: must abort, rewind, and retry cleanly.
- Regex rules can be brittle or over-trigger.
- Requires careful UX so users understand why the stream restarted.

**Verdict.** High value, large effort. One of omp's most distinctive features.

### 6.2 Model-specific prompt tuning

**What omp does.** omp adjusts prompts "relentlessly for each model" and reports large benchmark gains from format changes alone.

**mew's current state.** mew has a prompts crate and persona system, but no per-model prompt variants or automatic harness tuning.

**Pro adoption.**
- Free performance: same weights, better results.
- mew already supports multiple providers; model-specific prompts fit naturally.

**Anti adoption.**
- Requires ongoing measurement/benchmarking to avoid regressions.
- Prompt fragmentation increases maintenance.

**Verdict.** High value, medium effort. Start with per-provider prompt variants and measure.

---

## 7. Providers & routing

### 7.1 Model roles (`default`, `smol`, `slow`, `plan`, `commit`)

**What omp does.** Roles route work by intent. `default` for normal turns, `smol` for cheap subagent fan-out, `slow` for deep reasoning, `plan` for plan mode, `commit` for changelogs. Override at launch or mid-session.

**mew's current state.** mew has a provider-router (`crates/mew-provider-router`) that switches between a small and big model based on turn count and tool results. It also has reasoning variants and thinking variant switching. There is no explicit role system for subagents or plan mode.

**Pro adoption.**
- Uses the right model for the right job, saving cost.
- Fits mew's existing router architecture.

**Anti adoption.**
- Adds UI/config complexity.
- Requires users to configure multiple models.

**Verdict.** High value, small-to-medium effort. Extend the existing router with explicit roles.

### 7.2 Fallback chains

**What omp does.** Per-role fallback chains under `retry.fallbackChains`. If the primary throws 429s or quota errors, the next entry takes the rest of the turn.

**mew's current state.** mew does not have provider fallback chains.

**Pro adoption.**
- Reliability: sessions survive provider outages.
- Useful for users with multiple API keys.

**Anti adoption.**
- Model switching mid-turn can produce inconsistent output.
- Requires careful cost tracking.

**Verdict.** Medium-high value, medium effort.

### 7.3 Path-scoped models

**What omp does.** `enabledModels` and `disabledProviders` can be scoped to a `path:` prefix so different repos use different defaults.

**mew's current state.** mew loads config from a single global config file; personas can pin models, but there is no path-scoped model set.

**Pro adoption.**
- Different projects naturally need different models.
- Low friction once config supports it.

**Anti adoption.**
- Overlapping scopes are confusing.

**Verdict.** Small value, small effort.

### 7.4 Round-robin credentials

**What omp does.** Stack multiple API keys per provider; the runtime rotates with session affinity and per-credential backoff.

**mew's current state.** mew resolves one credential per provider via env var, keyring, or `credentials.json`.

**Pro adoption.**
- Distributes quota across keys.
- Useful for teams and high-volume users.

**Anti adoption.**
- Most users have one key per provider.

**Verdict.** Low-to-medium value, small effort.

---

## 8. Web & external tools

### 8.1 Built-in `web_search`

**What omp does.** `web_search` is built in, not bolted on. It chains 18 ranked providers and returns answer + citations. Site-aware extraction turns GitHub, arXiv, Stack Overflow, package registries, and docs into structured markdown.

**mew's current state.** mew has no web search tool. MCP servers could provide it, but it is not first-class.

**Pro adoption.**
- Models can research APIs, vulnerabilities, and papers without leaving the tool surface.
- Citation-aware output is useful for verification.

**Anti adoption.**
- Requires integrating many third-party search APIs.
- Without a default provider, the tool is dead code for most users.

**Verdict.** High value, large effort. A good candidate for a high-quality MCP tool or a built-in tool with one default provider.

### 8.2 Browser tool

**What omp does.** `browser` drives Puppeteer tabs over headless Chromium or any CDP-attached app (e.g., Slack). Stealth is on by default.

**mew's current state.** No browser tool.

**Pro adoption.**
- Fills gaps web search cannot: dynamic sites, authenticated pages, visual verification.

**Anti adoption.**
- Heavy dependency (Chromium/Puppeteer).
- Security and sandboxing concerns.

**Verdict.** Medium value, large effort. Lower priority than web search.

### 8.3 GitHub-as-filesystem (`pr://`, `issue://`, etc.)

**What omp does.** Internal URL schemes resolve transparently inside `read` and `search`: `read pr://1428`, `read issue://...`, `search` over diffs, `agent://<id>/findings.0.path` to pull JSON from subagents.

**mew's current state.** mew has no internal URL schemes beyond local paths. GitHub operations would require `bash` + `gh` or MCP.

**Pro adoption.**
- One tool interface the model already knows; no new GitHub-specific tools to learn.
- Clean abstraction for PRs, issues, subagent outputs, conflict files.

**Anti adoption.**
- Requires a virtual filesystem layer in `read`, `write`, `search`.
- Authentication needs careful handling.

**Verdict.** High value, medium effort. Fits mew's existing tool surface well.

### 8.4 Conflict resolution (`conflict://N`)

**What omp does.** Each merge conflict becomes a URL. The agent writes `@theirs`, `@ours`, or `@base` to `conflict://N` and the file resolves. Bulk form: `conflict://*`.

**mew's current state.** No equivalent.

**Pro adoption.**
- Turns git conflicts into a structured tool operation.
- Natural extension of a `pr://` / filesystem scheme layer.

**Anti adoption.**
- Requires parsing git conflict markers reliably.

**Verdict.** Medium value, small-to-medium effort if the scheme layer exists.

---

## 9. Collaboration & surfaces

### 9.1 `/collab` live sessions

**What omp does.** `/collab` puts the live session on a relay and returns a link and QR. Teammates join via `omp join` or a browser. Read-write or read-only links. Frames are sealed client-side.

**mew's current state.** mew already has a daemon + WebSocket protocol and a web UI. Shared sessions and multi-client support were recently implemented. A public relay would be a new feature, but the transport layer is mostly in place.

**Pro adoption.**
- Builds on mew's existing daemon/web architecture.
- Pair programming and demos.

**Anti adoption.**
- Relay infrastructure (hosting, abuse, e2ee) is non-trivial.
- Most users pair via screen sharing today.

**Verdict.** Medium value, large effort. Differentiator, but not urgent.

### 9.2 ACP (Agent Client Protocol) editor integration

**What omp does.** `omp acp` speaks the Agent Client Protocol over JSON-RPC so editors like Zed drive the same agent.

**mew's current state.** mew previously had ACP crates but removed them in favor of the daemon + WebSocket protocol (`mew-protocol`).

**Pro adoption.**
- Editor integration without a custom plugin.

**Anti adoption.**
- mew explicitly chose daemon/WebSocket over ACP.
- ACP is still an emerging standard.

**Verdict.** Low priority unless an important editor requires ACP.

### 9.3 SDK / RPC modes

**What omp does.** `omp --mode rpc` exposes NDJSON commands over stdio. Node SDK exposes `ModelRegistry`, `SessionManager`, `createAgentSession`.

**mew's current state.** mew has a TypeScript web client (`mew-web-client`) for the daemon protocol. No stdio RPC or Node embedding.

**Pro adoption.**
- Programmatic access unlocks integrations (Discord bots, CI, custom frontends).
- mew's daemon protocol could be exposed over stdio with a thin adapter.

**Anti adoption.**
- Another surface to maintain and test.

**Verdict.** Medium value, medium effort.

### 9.4 Shell completions generated from CLI metadata

**What omp does.** `omp completions` generates bash/zsh/fish completion scripts from live command/flag metadata. Model names and `--resume` values resolve dynamically.

**mew's current state.** mew uses `clap` but does not ship a completions subcommand.

**Pro adoption.**
- Small, high-polish feature.
- `clap` has built-in completion support.

**Anti adoption.**
- None significant.

**Verdict.** Small value, tiny effort. A nice quick win.

---

## 10. Workflow helpers

### 10.1 `omp commit` atomic splits

**What omp does.** Reads the working tree via git tools, splits unrelated changes into atomic commits ordered by dependencies, rejects cycles, scores source files above tests/docs/configs, and excludes lock files from analysis.

**mew's current state.** mew has no commit helper. Users run `git` via `bash`.

**Pro adoption.**
- Saves time on cleanup commits.
- Atomic commits are better for review.

**Anti adoption.**
- Easy to get wrong and produce surprising commits.
- Requires strong trust before users let the agent commit.

**Verdict.** Medium value, medium effort.

### 10.2 `/review` with P0-P3 verdicts

**What omp does.** Spawns reviewer subagents that sweep branches/commits/uncommitted work in parallel. Returns a clear verdict with P0-P3 issues and confidence scores.

**mew's current state.** No equivalent.

**Pro adoption.**
- Code review is a natural agent task.
- Fits mew's subagent infrastructure.

**Anti adoption.**
- Quality depends heavily on the reviewer model and prompts.
- False positives erode trust.

**Verdict.** Medium value, medium effort. A good subagent use case.

### 10.3 Import existing configs (Cursor, Cline, Codex, etc.)

**What omp does.** On first run, omp reads rules/skills/MCP servers from `.claude`, `.cursor`, `.windsurf`, `.gemini`, `.codex`, `.cline`, `.github/copilot`, `.vscode` in their native shapes.

**mew's current state.** mew reads `AGENTS.md` / `CLAUDE.md` via `mew-context`, personas from `.mew/personas`, MCP from `mcp.json`, and config from `~/.config/mew/config.toml`. No import from competing tools.

**Pro adoption.**
- Lowers switching cost for new users.
- Most teams already have rules in one of these formats.

**Anti adoption.**
- Each format is slightly different; "native" support can mean fragile parsing.
- mew's AGENTS.md approach is already standard for opencode.

**Verdict.** Medium value, medium effort. Good onboarding project.

### 10.4 `ask` tool for structured questions

**What omp does.** The `ask` tool renders an option picker mid-turn. Same picker surfaces over ACP.

**mew's current state.** mew has `AskUser` tool and `AgentEvent::AskUser` wiring through TUI and web UI.

**Pro adoption.**
- mew is already close; the gap is mostly prompt/schema tuning.

**Anti adoption.**
- None.

**Verdict.** Small value, tiny effort. Verify the UX is as smooth as omp's picker.

### 10.5 `todo` / `job` tools

**What omp does.** `todo` mutates an ordered session todo list with phase tracking. `job` waits on or cancels background jobs.

**mew's current state.** mew already has `TodoCreate/Update/Complete/Delete/ListTool` and `ShellBackground/ShellMonitor/JobStatus/JobBlock/JobCancel`. This is largely parity.

**Pro adoption.**
- Continue refining these; they are already present.

**Anti adoption.**
- None.

**Verdict.** Maintain parity. No major new work needed.

---

## 11. Native quality-of-life modules

omp ships roughly 55k lines of Rust in modules that other harnesses shell out for. Many are not "features" but implementation-quality improvements:

| omp module | what it does | mew state | relevance |
|------------|--------------|-----------|-----------|
| shell | embedded bash | forks bash | high |
| grep | in-process regex search | shells out to rg/grep | high |
| keys | kitty keyboard protocol | unknown | low |
| text | ANSI-aware width/wrap | partial | medium |
| summary | tree-sitter structural summaries | none | high |
| ast | ast-grep structural rewrites | none | medium |
| fs_cache | mtime-keyed file cache | none | high |
| highlight | syntect highlighting | yes (`ratatui-mdstream`) | parity |
| pty | native PTY | none | medium |
| glob | in-process glob | yes (`ignore`/`globset`) | parity |
| workspace | walker + AGENTS.md discovery | yes (`mew-context`) | parity |
| appearance | dark/light mode detection | unknown | low |
| power | macOS power assertions | none | low |
| task | blocking work on thread pool | partial | medium |
| fd | filesystem walker | none | medium |
| iso | workspace isolation (APFS/btrfs/ZFS reflinks) | none | low |
| prof | profiler / flamegraphs | none | low |
| ps | cross-platform process-tree kill | partial | medium |
| clipboard | system clipboard | none | medium |
| tokens | BPE token counting | none | medium |
| sixel | terminal image rendering | none | low |
| html | HTML-to-markdown | none | high |

The highest-leverage items for mew are: **fs_cache** (shared file cache for read/grep/LSP), **html** (for web search and URL reading), **text** (ANSI width utilities), and **tokens** (accurate context accounting).

---

## Prioritized shortlist

| Priority | Feature | Effort | Why now / why later |
|----------|---------|--------|---------------------|
| P0 | In-process `grep` | Small | Removes `rg` dependency, improves cancellation, fits mew's Rust stack. `glob` is already in-process; this is the obvious next step. |
| P0 | Shell completions subcommand | Tiny | Pure polish, `clap` supports it directly. |
| P0 | Improve `edit` tool (multi-line anchors + stale-file hints) | Small | Addresses the most common edit failures without inventing a new DSL. |
| P1 | Model roles (`smol`/`slow`/`plan`) | Small-Medium | Extends existing provider-router. Saves cost and improves subagent quality. |
| P1 | Summarizing `read` | Small-Medium | Heuristic summaries first; tree-sitter later. Keeps context windows manageable. |
| P1 | `fs_cache` shared file cache | Medium | Speeds up repeated `read`/`grep`/`glob` and prepares ground for LSP. |
| P1 | Internal URL schemes (`pr://`, `issue://`, `agent://`) | Medium | Reuses existing `read`/`search` tools; unlocks GitHub and subagent introspection. |
| P1 | HTML-to-markdown | Small | Unlocks reading web pages, docs, and search results cleanly. |
| P2 | Hashline edits | Medium-Large | High value but requires a new patch DSL and model retraining/coercion. |
| P2 | Hindsight session summaries | Medium-Large | Cross-session memory is a major differentiator, but quality is hard. |
| P2 | Time-traveling stream rules | Large | Distinctive and powerful, but complex streaming implementation. |
| P2 | `task` subagents with typed JSON results | Medium | Natural extension of existing subagent work. |
| P2 | Advisor/reviewer subagent | Medium | Can be built on subagents; token cost is the main concern. |
| P3 | LSP integration | Large | Huge value, but heavy. Per your note, defer until core tool harness is solid. |
| P3 | DAP/debugger | Large | Niche and heavy. Defer unless debugging becomes a focus. |
| P3 | Browser tool | Large | Useful but heavy dependency. |
| P3 | `eval` persistent Python/JS cells | Large | Powerful, but large security/sandbox surface. |
| P3 | `/collab` public relay | Large | Builds on existing daemon but needs hosting/e2ee. |
| P3 | ACP editor protocol | Medium | mew intentionally moved away from ACP. Revisit only if required by an editor. |

---

## Appendix: sources and mew files

**omp sources**
- Homepage: https://omp.sh
- Repository: https://github.com/can1357/oh-my-pi
- Blog post on harness tuning: https://blog.can.ac/2026/02/12/the-harness-problem/

**Relevant mew files**
- `crates/mew-tools/src/tools/edit.rs` — current edit tool
- `crates/mew-tools/src/tools/bash.rs` — current bash tool
- `crates/mew-tools/src/tools/grep.rs` — current grep tool (shells out)
- `crates/mew-tools/src/tools/glob.rs` — current glob tool (in-process)
- `crates/mew-tools/src/tools/subagent.rs` — subagent schema tool
- `crates/mew-provider-router/src/lib.rs` — provider routing
- `crates/mew-session/src/lib.rs` — session persistence
- `crates/mew/src/main.rs` — tool registration, CLI, agent builder
- `crates/mew-agent/src/agent.rs` and `turn.rs` — agent loop
- `crates/mew-protocol/src/lib.rs` — wire protocol
- `CLAUDE.md` — project architecture overview
