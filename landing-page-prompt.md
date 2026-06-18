# mew landing page prompt

design a single-page landing site for mew, a terminal-native AI coding agent written in Rust.

## vibe

spare, confident, a little playful. think: the aesthetic of a well-maintained open-source tool that doesn't need to convince you it's serious. monospace where it counts, not everywhere. dark background. one accent color (something warm — amber, rust, or gold). no gradients, no glassmorphism, no animations that exist just to animate. the page should feel like it loads in 50ms and respects the user's time.

## structure (top to bottom, single scroll)

**hero**
- the word "mew" in large type. under it, one line: "a terminal agent. fast, local, yours."
- a short paragraph (2-3 sentences max) that says what it does without buzzwords. mention: streams LLM responses, runs tools, edits code, respects your permissions.
- two buttons: "get started" (links to install/docs) and "view source" (links to github). the "get started" button has the accent color. "view source" is a ghost button.
- a subtle terminal-cursor blink after the hero text. not a full fake terminal. just a cursor.

**what it looks like**
- not a screenshot in a browser mockup. show a clean terminal window frame with a real-looking session: a user prompt, a streaming assistant response mid-sentence, a tool call with a spinner, a permission dialog. caption under it: "same terminal you already live in. no web app, no electron, no telemetry."
- the terminal frame should have a subtle shadow or border but feel lightweight.

**three-column grid: principles**

each column has a small monospace icon or label, a title, and one sentence.

column 1 — **provider-agnostic**. "bring your own keys. openai-shape, anthropic-shape, or auto-route between them. the model catalog stays fresh."

column 2 — **tools with guardrails**. "bash, file editing, code search. every dangerous action hits a permission gate you control. declarative rules in toml."

column 3 — **local & yours**. "sessions are jsonl on your disk. no analytics, no cloud, no phoning home. grep your own agent history."

**feature highlights (alternating horizontal sections)**

section a — "subagents that work in parallel"
- left: a small ascii-style diagram showing parent agent → two child agents running simultaneously
- right: text explaining subagents get their own sessions, restricted toolsets, wall-clock caps. mention the `exit_tool` pattern. don't over-explain.

section b — "plugins and hooks"
- right: a tiny code snippet showing a hook definition (3-4 lines of toml or rust)
- left: text about the plugin system — intercept turns, rewrite tool args, inject system prompts, register slash commands. mention wasm runtime.

section c — "acp: drive it from anywhere"
- left: a simple two-node diagram (laptop ↔ server) with "ACP" in the middle
- right: "mew talks ACP natively. use it as a client to external agents, or expose your agent as a server for zed, neovim, or whatever speaks jsonrpc."

**pricing / cta**
- no pricing table. it's open source. just a centered line: "mit licensed. free. always."
- below it: "works with opencode zen, z.ai, deepseek, anthropic, openai, or anything with an http endpoint."
- a final "get started" button.

**footer**
- minimal: project name, link to github, link to docs, link to discord or whatever community channel exists. no copyright line in giant text.

## technical notes for implementation

- one html file, minimal css. no javascript unless the cursor blink needs it (and even then, a css animation is better).
- system font stack. no webfonts.
- the terminal screenshot should be actual text rendered in a styled div, not an image, so it stays sharp on any display and loads instantly.
- the ascii diagrams should be `<pre>` blocks with the mono accent color on key parts.
- responsive but mobile-second. the primary layout target is a laptop/desktop screen. on mobile the three-column grid stacks. the alternating sections stack image-above-text.
- total page weight target: under 50kb uncompressed including all css. no images. no tracking. nothing from a cdn.

## anti-brief

do not: use the words "revolutionize," "empower," "next-generation," "seamless," "unleash," or "supercharge." do not use emoji. do not put a "trusted by" section with made-up company logos. do not animate scroll reveals. do not use a cookie banner (there are no cookies). do not write "in conclusion" or "to sum up." do not use three words where one will do.
