# content brief: mew landing page

## logo
ascii + japanese, monospace.

╱|、
(˚ˎ 。7
|、˜〵
じしˍ,)ノ

## the problem with the current prompt

the existing `landing-page-prompt.md` is well-intentioned — it bans the worst marketing habits, limits scope, and makes good technical choices. but it still structures the page like a product landing page: hero → principles → features → CTA → footer. that's the squarespace-for-startups playbook. it works for products that need to explain themselves. mew doesn't.

mew is a developer tool. the people who'd use it will know within 10 seconds whether they want to try it. the page's job is to show them what it is and how to get it. everything else is noise.

the current prompt also hedges against hype while still doing some of it. "the bus factor is you" is a clever line, but it's still positioning. "gated, not neutered" is a tagline disguised as a section header. "bring your own keys" is marketing copy for "you need your own API keys." these are small things, but they add up to a page that feels like it's trying to sell you something, even if it's selling you on "we don't sell things."

## direction

replace the structured landing page with something closer to a well-set zine or README. the page is three things: what it is (with a live demo), what it does (three short paragraphs), and how to get it. everything else is links.

the terminal demo is the entire pitch. everything else is logistics.

### on feature differentiation

don't position ACP, MCP, or subagents as differentiators. every harness has them now. listing them as features reads like you're trying to sell someone on a spec sheet — which is exactly what this page shouldn't do. mention them in the third paragraph where they belong: as extension points, not as reasons to switch.

the subprocess plugin runtime, on the other hand, is actually different. but don't lead with the architecture — lead with what the reader can do with it. plugins can add custom tools, hook into permissions, filter shell environment, inject content into the TUI. the companion in the TUI runs on this same protocol, which proves it's real without making a digital pet the headline of your extensibility story.

## tone

write like you're explaining the tool to a coworker over a screen share. short sentences. be excited about what the tool can do, but ground it in specifics — show what's good rather than announcing it. confidence reads as enthusiasm without needing to perform it.

no marketing vocabulary. not even the restrained kind. "fast" is fine. "blazing fast" is not. "local" is fine. "privacy-first" is not. if you find yourself reaching for a compound modifier, rewrite the sentence instead.

the page should feel like it was written by someone who builds the tool, not someone hired to market it.

## page shape

one scroll. four sections, maybe five. no numbered principles, no feature grid, no alternating layout. just text and one terminal demo.

that's it. the page ends when the content ends.

## section by section

### 1. top

**logo:** use "mew" in the monospace font stack. large but not hero-sized — maybe 2rem. a subtle ascii border or decoration is fine if it adds character without taking space. one idea: the mew name rendered in a block-character style, 2-3 lines of monospace art. but keep it small. the logo is not the point.

no tagline, no slogan, no calling card. the terminal demo says everything a tagline would. the page opens with the logo, the install command, and the demo. if someone needs "a terminal agent for code" explained to them, they're not the audience.

### 2. terminal demo

this is the whole page. everything before it is 4 lines of text. everything after it is logistics.

a css-styled `<div>` that looks like a terminal window. no chrome — no macos traffic lights, no fake title bar. just a dark rectangle with a monospace font, slightly inset from the page edges, with maybe a subtle border or shadow.

inside it, a realistic session transcript. this should look like something a real user would see. the content should be grounded in what mew actually does — real tool names, real permission prompts, real output. specific example:

```
~/code/myapp $ mew "convert the auth module to argon2"

thinking...

the current bcrypt implementation lives in src/auth/password.rs.
let me look at the hashing setup first.

○ grep -n "bcrypt" src/auth/
  src/auth/password.rs:4:use bcrypt::{hash, verify, BcryptError};
  src/auth/password.rs:18:    hash(plain, DEFAULT_COST)?
  src/auth/password.rs:24:    verify(plain, hashed)?

allow write src/auth/password.rs? [y/n] y

○ write src/auth/password.rs
  - use bcrypt::{hash, verify, BcryptError};
  + use argon2::{Argon2, PasswordHash, PasswordVerifier, PasswordHasher};

the module uses argon2 now. the verify call returns Result<(), _>
so the error handling stays the same — i just swapped the hash/verify
functions. tests pass.
```

this is a real interaction. the tool calls show what mew actually does. the permission prompt shows what the user sees. the final message is the kind of summary a good agent gives — direct, technical, no fluff.

the text should appear to be streaming. use a css animation that types the last few characters with a blinking cursor. the rest of the text is static (already "typed"). the cursor blinks at the end. this is the only animation on the page.

below the terminal, a small line in regular text:

```
brew tap mew/tap && brew install mew
```

this is the only CTA. below it, smaller: `source` · `docs` · `community`

that's the whole first screen. if someone scrolls no further, they know what mew is, what it looks like, and how to install it.

### 3. three paragraphs

below the fold. three short paragraphs, each 2-4 sentences. no headers, no icons, no columns. just text with some vertical space between them.

**paragraph 1 — what it is:**

describe the agent loop. it takes your prompt, streams a response from an LLM, and can run tools along the way — bash commands, file edits, code search. each turn the agent decides what to do next. sessions are stored as JSONL files on your disk, so you can grep your own history.

**paragraph 2 — models and config:**

you bring your own API keys. openai-compatible and anthropic-compatible endpoints both work. config is a TOML file. permissions are declarative — you can allow specific tools for specific paths, or prompt on everything. the model catalog comes from models.dev and refreshes itself.

**paragraph 3 — extending it:**

MCP servers register automatically from `mcp.json`. ACP lets you expose the agent as a JSON-RPC server so editors like Zed or Neovim can drive it directly. subagents run in parallel with their own sessions and tool restrictions.

but the real extensibility is plugins. plugins can add custom tools, intercept permission decisions, filter shell environment, inject content into the sidebar, or register slash commands — all over JSON-RPC on stdin/stdout, with persistent namespaced storage. the companion in the TUI (a sprite with speech bubbles and persistent state) runs on this same protocol — it's a plugin, not a built-in, which means the runtime is real and any third party can use it. plugins live in `~/.config/mew/plugins/` and `<project>/.mew/plugins/`, discovered at startup and sorted for deterministic hook ordering.

the `Dispatcher` trait that plugins implement doesn't change between transports. the subprocess runtime is what ships today, but the same trait could run under wasmtime without touching any other code.

each paragraph should feel like documentation, not a pitch. if someone reads all three, they should know what the tool does and how to customize it, without feeling like they were sold anything.

### 4. bottom

a simple separator line (an ascii rule: `──────` or similar).

centered, small text:

```
MIT licensed. works with opencode zen, z.ai, deepseek, anthropic, openai.
```

no "free. always." — MIT license is the statement, you don't need to repeat it.

a final install button:

```
brew tap mew/tap && brew install mew
```

same as the one at the top. repetition is fine here — it's the only action item.

### 5. footer

three links, inline:

```
source · docs · community
```

no copyright line. no "made with" anything. just the links.

## what not to include

- **no "what makes it different" section.** the product is different because it's a local CLI tool. that's obvious from the first sentence.
- **no "principles" or "values" section.** principles are for companies. this is a tool.
- **no feature grid.** the three paragraphs cover what it does. anything more detailed belongs in docs.
- **no ascii art diagrams of subagents or ACP.** those are architecture details, not landing page content.
- **no alternating left-right layout.** that's a template pattern for filling space.
- **no competitive comparisons.** not even implied ones. no "why not just use X."
- **no pricing section.** MIT license.
- **no changelog, no version number, no "what's new."**
- **no "get started" and "view source" buttons.** the install command and footer links cover both.

## what the copy should NOT sound like

- not like a YC application
- not like a product hunt launch
- not like a blog post titled "why we built..."
- not like documentation (this is a landing page, not a README — it needs to *feel* good, not just convey information)
- not like a tweet thread about the future of developer tools
- not like the existing prompt's copy, which is restrained but still has the cadence of marketing ("the bus factor is you", "gated, not neutered", "bring your own keys")

it should sound like a tool author describing their tool to someone who might use it. direct, specific, confident in what the tool does without needing to sell it. the kind of thing where you finish reading and think "okay, let me try it" — not "wow, what a great vision."

## accent color

pick one and use it for: the blinking cursor, links, the install command highlight, and subtle borders. the amber-to-rust range is right. something like `#d4a574` or `#c0785c` — warm, not neon. avoid pure gold (`#ffd700` territory) which reads as luxury brand. you want something that looks like aged paper or old terminal phosphor.

## content density

the page should feel like it has room to breathe. generous padding between sections. the terminal demo should be the densest element on the page — everything else is sparse by comparison. white space is not wasted space here; it's the difference between "landing page" and "documentation."

if the page feels short, it's the right length. the target audience will scroll once, see the terminal demo, read two paragraphs, and install it. design for that reader.

## technical constraints

carry over from the original prompt (these are good and should not change):

- single HTML file, embedded CSS, no JavaScript (CSS-only streaming animation preferred)
- system font stacks. no web fonts
- under 50KB
- dark background, one accent color
- no tracking, no cookies, no CDN
- responsive but desktop-primary
- no scroll-reveal, no hover effects, no transitions over 150ms
- section IDs for fragment linking

additions:

- the terminal demo is the visual centerpiece. give it real CSS attention: proper line-height (1.5-1.6), slight letter-spacing, a background that's a slightly different shade from the page background
- the streaming animation should be slow enough to read (maybe 40-60ms per character for the last line) but fast enough to not annoy (finish in under 3 seconds)
- use CSS custom properties for the accent color so it's easy to tweak
- the ascii rule separators between sections should use simple dashes or box-drawing characters — no illustrated unicode dividers

## reference: what mew actually does

(facts from the codebase, for copywriting grounding)

**tools available:** bash, read, write, edit, glob, grep, echo, ask_user, exit_tool, progress_update, todo (create/update/complete/delete/list), flag_important, skill, subagent_start, subagent_wait. MCP tools register dynamically.

**permissions:** three sensitivity levels — ReadOnly (auto-allow), Mutating (prompt), Dangerous (prompt). declarative rules in `config.toml` override defaults per tool/path pattern.

**models:** openai-shape and anthropic-shape adapters. built-in defaults: opencode-zen, opencode-go, z-ai, deepseek. model catalog from models.dev with 24-hour cache. custom providers via config.

**sessions:** JSONL files at `~/.local/share/mew/sessions/<id>/`. metadata in `meta.json` alongside. subagent sessions link to parents.

**config:** `~/.config/mew/config.toml` (or platform equivalent). credentials via env var, system keyring, or `credentials.json`. environment variables with `MEW_` prefix override config.

**extensibility:** MCP servers from `mcp.json` (Claude Code format). ACP server mode for editor integration. skills from `.mew/skills/` or `.opencode/skills/`. subprocess plugin runtime: JSON-RPC 2.0 over stdin/stdout, 14 hook points (`on_system_prompt`, `on_chat_message`, `on_chat_headers`, `on_chat_params`, `on_tool_execute_before`, `on_tool_execute_after`, `on_permission_ask`, `on_shell_env`, `on_turn_end`, `on_event`, `on_register_tools`, `on_register_slash_commands`, `execute_slash_command`, `init`/`shutdown`). plugins get persistent namespaced storage, TUI injection via `set_ui`, and bidirectional host calls (`host-notify`, `host-config-read`, `host-log`, `host-storage-read/write/delete`, `host-set-ui`). discovery in `~/.config/mew/plugins/` and `<project>/.mew/plugins/`. `Dispatcher` trait is transport-agnostic — wasmtime support is a drop-in without changing agent code.

**CLI:** `mew` starts interactive chat. `mew run "prompt"` runs non-interactively. `mew acp` starts ACP server. `mew config show|edit|path` for configuration.

**license:** MIT.
