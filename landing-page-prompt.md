# mew landing page

single-page site. dark background, one accent color (amber/rust/gold range — pick one and stick to it). monospace only where it makes sense (code blocks, terminal frame, labels). system font stack everywhere else. no webfonts, no cdn dependencies, no tracking, no cookies.

the page should feel like it loads before your finger leaves the key. target: under 50kb uncompressed, html + css in one file.

## structure

### hero

large type: **mew**

one line under it: "a terminal agent. fast, local, yours."

paragraph (2-3 sentences): describe what it does without adjectives that need air quotes. it streams llm responses in your terminal, runs bash commands, edits files, searches code. every dangerous action hits a permission prompt you control. sessions live on your disk as jsonl. nothing leaves your machine unless you point it at a model.

below the paragraph, a small code block showing the install command:

```
curl get.mew.computer | bash
```

two buttons: **get started** (filled, accent color, links to docs/install) and **view source** (outline/ghost, links to github).

after the hero text, a single blinking cursor. css animation, no js. not a fake terminal — just a cursor at the end of the last line, like the page is still listening.

### what it looks like

a terminal window frame rendered as a styled div (not an image). inside it, a real-looking session:

- a prompt line: `~ mew "refactor the auth module to use argon2"`
- a streaming assistant response, cut mid-sentence: `Thought for a second… the current bcrypt implementation lives in`
- a tool call with a spinner: `◌ grep "bcrypt" src/auth/`
- a permission dialog: `allow grep src/auth/*.rs? [y/n/a] y`

caption: "the terminal you already live in. no electron, no web app, no telemetry."

the terminal frame needs enough css detail to feel alive — correct line-height, letter-spacing, and a font that actually ships on the system (sf mono / consolas / dejavu sans mono). not uncanny-valley crisp.

### three principles

three columns. each has a small monospace label, a title, and one sentence.

**column 1** — label: `any model`. title: "bring your own keys." "openai-shape, anthropic-shape, or auto-route between them. model catalog stays fresh from models.dev."

**column 2** — label: `tools`. title: "gated, not neutered." "bash, file editing, code search — all available, all permission-gated. declare rules in toml. prompt-once or prompt-always."

**column 3** — label: `local`. title: "yours, full stop." "sessions stored as jsonl on your disk. grep your own agent history. no analytics, no cloud, no phoning home."

### what makes it different

a short centered section between principles and features. one or two sentences that answer "why not just use claude code / cursor / copilot":

"mew is a tool you install, not a service you subscribe to. it doesn't know your name, doesn't cache your code, and works the same offline as online. the bus factor is you."

### features (alternating horizontal)

**subagents that work in parallel** — left: ascii diagram showing parent agent → two children running simultaneously. right: text. "spawn subagents with restricted toolsets and wall-clock caps. each gets its own session. they report back via the `exit_tool` pattern. run a researcher and a reviewer in parallel while you work on something else." the diagram must be legible in under two seconds or it's not earning its space.

**plugins and hooks** — right: a 3-4 line code snippet showing a hook definition (toml or rust). left: text. "intercept turns, rewrite tool arguments, inject system prompts, register slash commands. plugins run in a wasm runtime with defined interfaces. nothing escapes the sandbox unless you let it."

**acp: drive it from anywhere** — left: two-node diagram (laptop ↔ server) with "acp" between them. right: text. "mew speaks acp natively. use it as a client to external agents, or expose your agent as a server. zed, neovim, custom tooling — if it speaks jsonrpc, it can drive mew."

### bottom

centered. no pricing table, no tiers, no "contact sales."

"mit licensed. free. always."

"works with opencode zen, z.ai, deepseek, anthropic, openai, or anything with an http endpoint and a compatible api shape."

a final **get started** button.

### footer

project name, link to github, link to docs, link to discord (or whatever community channel exists). no copyright novel. no "all rights reserved" in 14pt. just the links.

## technical constraints

- one html file, embedded css. no javascript unless the cursor blink can't be done with css (it can: `@keyframes blink` on a border or pseudo-element).
- system font stack: `-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Oxygen-Sans, Ubuntu, Cantarell, sans-serif` for body; `"SF Mono", "Cascadia Code", "Consolas", "DejaVu Sans Mono", monospace` for code.
- the terminal frame is styled html, not an image. real text stays sharp at any resolution.
- ascii diagrams in `<pre>` blocks. key parts get the accent color via inline spans or css classes.
- responsive but not mobile-first. desktop is the primary target. on mobile: three columns stack, alternating sections stack image-above-text. don't break a sweat over mobile — the audience is on a laptop.
- all sections have id attributes so fragment links work (`#features`, `#acp`, etc).
- no scroll-reveal animations, no hover effects that exist just to hover, no transitions longer than 150ms.
- no cookie banner. there are no cookies.
- do not use the words: revolutionize, empower, next-generation, seamless, unleash, supercharge, game-changer, enterprise-grade, robust, cutting-edge, leverage, utilize.
- no emoji anywhere on the page.
- no "trusted by" logos. no fake testimonial quotes.
- no "in conclusion" or "to sum up." the page ends when the content ends.
