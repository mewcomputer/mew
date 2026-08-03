# Plan: Qwen3.8 thinking variants + token-budget slider

**Goal:** Give `qwen3.8-max` and `qwen3.8-max-preview` configurable thinking — effort levels `low` / `medium` / `xhigh` plus a token-budget slider shown alongside the thinking selection in all three frontends (TUI, web UI, GPUI desktop).

**Sequencing (revised):** Foundations first (catalog → config → resolution → protocol), then the **TUI end-to-end with extended controls (keyboard + mouse + number input) and a verification checkpoint**, then web UI and desktop GPUI, then full verification. Rationale: the TUI is the cheapest place to prove the whole pipeline (resolution, protocol, slider UX) before duplicating it in two other frontends.

**Key constraint (from QwenCloud docs):** `thinking_budget` is NOT supported by every thinking model (only Qwen3.8/3.7/3.6/3.5/3-VL/3, GLM, Kimi series), and for qwen3.8-max it is mutually exclusive with `reasoning_effort` — setting both errors. So the slider is gated by per-model capability metadata and only models that declare a budget range get it. Effort variants map to budgets: low→4096, medium→16384, xhigh→262144; model default is budget 131072 / effort xhigh. `qwen3.8-max*` enables thinking by default, so an explicit "off" must send `enable_thinking: false` (plain None leaves thinking on).

## Phase A: Foundations

### A1. Catalog metadata (`crates/mew-catalog/src/lib.rs`)

- Add `ThinkingBudget { min, max, step, default, by_effort: Vec<(String, i64)> }` (serde, `Default`).
- Add `Model.thinking_budget: Option<ThinkingBudget>` (serde default, skip if none).
- Add `Catalog::thinking_budget(&self, model_id) -> Option<ThinkingBudget>` mirroring `thinking_variants`: model field first, then a builtin rule.
- Builtin rule: `id.contains("qwen") && (id.contains("3.8") || id.contains("3-8"))` → `ThinkingBudget { min: 0, max: 262144, step: 1024, default: 131072, by_effort: [("low",4096),("medium",16384),("xhigh",262144)] }`. (Matches both `qwen3.8-max` and `qwen3.8-max-preview`, incl. dated snapshots.)
- In `builtin_thinking_variants`, add a qwen3.8 rule **before** the "no configurable thinking" bucket (which currently returns empty for all `qwen` ids):
  - variants `low`/`medium`/`xhigh` with params `{"enable_thinking": true, "reasoning_effort": "<level>"}`, plus an explicit `off` variant with params `{"enable_thinking": false}`.
- `default_thinking`: extend preference chain to `"high"` → `"thinking"` → `"xhigh"` → first. qwen3.8 defaults to `xhigh` (matches API default); existing models unaffected (they all have `high` or `thinking`).
- `map_variant` (model-switch carry-over): when the source name is `budget:<n>` and the target model has `thinking_budget`, carry it over clamped/snapped to the target range. Also make the closest-effort tie-break prefer the higher level (docs map OpenAI `high`→`xhigh`, and `min_by_key` currently returns the lower on ties).
- Unit tests: qwen3.8 variants (names, params, off variant), `thinking_budget` rule, `default_thinking` → xhigh, `map_variant` budget carry-over + `high`→`xhigh`, non-qwen3.8 models unchanged.

### A2. Config override (`crates/mew-config`, `crates/mew/src/setup/providers.rs`)

- `CustomModel.thinking_budget: Option<mew_catalog::ThinkingBudget>` (serde default) so users can add/override budget ranges via `config.toml`.
- Thread through `build_custom_model` (explicit value, or `base` value when `merge`).
- Add a catalog test asserting a custom model with only `thinking_budget` set still gets built-in effort variants (variants and budget are independent axes).

### A3. Resolution (`crates/mew/src/setup/providers.rs`)

- Change `resolve_reasoning` to return `Option<(ReasoningConfig, String)>` (config + canonical resolved name). Handle:
  - `"off"` / `"none"`: if the model defines an explicit off variant, return its params (→ `enable_thinking: false`); else `None`.
  - `"budget:<n>"`: parse `n`, clamp to the model's budget `min..=max`, snap to `step`, return params `{"enable_thinking": true, "thinking_budget": n}` and canonical name `budget:<n>`; no budget metadata or unparseable → `None` (treated as unknown variant, same as today).
  - anything else: current lookup by name (params unchanged).
- `ReasoningConfig`/`Request` docs in `crates/mew-provider/src/lib.rs`: add a qwen example (`{"enable_thinking": true, "thinking_budget": 8192}`).
- Update call sites:
  - `crates/mew/src/runtime/local.rs::set_thinking`: stop short-circuiting off before resolution — call `resolve_reasoning` for every non-empty name (empty → None).
  - `crates/mew/src/commands/daemon.rs` thinking_setter: same; return the canonical resolved name (clamped budget) instead of the raw input.
  - `crates/mew/src/commands/tui.rs` startup path: use the resolved name for `app.active_thinking_variant`.
- Tests in `providers.rs` (follow existing style ~line 1157): budget parse/clamp/snap, off→`enable_thinking:false` for qwen3.8, off→None for models without an off variant, unknown budget → None.

### A4. Protocol (`crates/mew-protocol/src/lib.rs`, `mew-web-client/src/index.ts`)

- Add `ThinkingBudgetInfo { min, max, step, default, by_effort: Vec<(String,i64)> }` to the protocol (struct + JSON roundtrip test).
- `ModelInfo.thinking_budget: Option<ThinkingBudgetInfo>` (serde default/skip).
- No new message types: budget rides the existing `SetThinkingVariant { variant }` as the string convention `"budget:<n>"`; document it next to `ThinkingVariantInfo` and in `setThinkingVariant` docs.
- Daemon model-list builder (`crates/mew/src/commands/daemon.rs`) populates `thinking_budget` from `cat.thinking_budget(&m.id)`.
- TS: add `ThinkingBudgetInfo` and `ModelInfo.thinking_budget?: ThinkingBudgetInfo | null`; add `setThinkingBudget(tokens: number)` sugar that calls `setThinkingVariant(\`budget:${tokens}\`)`.

## Phase B: TUI (proving ground — do first, verify, then port)

### B1. TUI state and population (`crates/mew-tui`)

- `App`: add `thinking_budget: HashMap<String, mew_protocol::ThinkingBudgetInfo>` (keyed by bare model id, like `thinking_variants`) and `budget_draft: Option<String>` (the budget value being typed/stepped; `None` when the budget row is not selected or no budget metadata exists). Populate `thinking_budget` in `app/mod.rs` `ModelList` handler (daemon mode) and `commands/tui.rs` (~line 620, local mode). `thinking_variants` type stays `Vec<String>` (no churn).
- `app/pickers.rs::open_thinking_variant_picker_for`: when the model has budget metadata, append a budget row (id `"budget"`, label `"token budget"`) after the variant rows. Seed `budget_draft` from the active `budget:<n>` variant if set, else the `by_effort` mapping of the active effort, else `default`. Clear `budget_draft` in `close_picker`.

### B2. TUI budget row rendering (`crates/mew-tui/src/ui/overlays.rs::draw_picker`)

- Render the budget row as a track line, e.g. `[█████░░░░░░░] 8192 tok`, computed from min/max/step/draft. When the user is typing digits (draft is being edited), show the typed value with a cursor underscore instead of the track.
- Record the track's screen rect on `App` (`picker_budget_rect: Option<Rect>`, set during draw, cleared when the picker closes) so `handle_mouse_event` can hit-test it.

### B3. TUI input handling (`crates/mew-tui/src/events.rs`)

Keyboard (`handle_picker_key`, only when the budget row is selected in a `thinking_variant` picker):
- `KeyCode::Left` / `Right`: adjust the draft by `step`, clamped to `min..=max` and snapped to `step` (seed from current draft if unset). **Note:** Left/Right currently move the filter cursor — that must remain the behavior for all other rows.
- Digit chars (`0`-`9`): number input. If the draft equals the seeded value, typing replaces it; otherwise appends. (Letters still filter the picker as today.)
- `Backspace`: pop the last digit; empty draft reseeds to the metadata default.
- `Enter`: commit `SetThinkingVariant("budget:<n>")` and close the picker (existing Enter path — add a `"budget"` arm alongside `"thinking_variant"`).
- `Esc`: closes the picker (existing behavior; typed draft is discarded).

Mouse (`handle_mouse_event` — currently gated to `Normal | SlashCommand`; extend to `CommandPalette` when the picker kind is `thinking_variant` and `picker_budget_rect` is set):
- `Down(Left)` on the track: map column → value (`min + frac * (max-min)`, snapped to step), set the draft (no commit yet).
- `Drag(Left)`: update the draft as the pointer moves (preview).
- `Up(Left)`: commit `SetThinkingVariant("budget:<n>")` from the draft. **Do not close the picker** — the user may keep dragging or pick a variant.
- `ScrollUp`/`ScrollDown` over the track: nudge the draft by `±step` and commit immediately (single discrete event, no spam).
- Other rows/areas in picker mode: ignore (unchanged behavior; pickers were mouse-inert before).

Status pill (`crates/mew-tui/src/ui/status.rs`): strip the `budget:` prefix (show the number).

### B4. TUI tests (`crates/mew-tui/src/app/tests.rs`)

- Budget row appears only when metadata is present.
- Draft seeding: active `budget:<n>` wins, then `by_effort[active effort]`, then `default`.
- Left/Right adjusts by step and clamps at min/max.
- Digit typing replaces the seed, appends after a nudge; Backspace pops; empty reseeds.
- Enter emits `Action::SetThinkingVariant("budget:<n>")` and closes.
- Mouse: set `picker_budget_rect`, synthesize `MouseEvent` Down/Drag/Up/Scroll, assert draft updates and Up commits the right variant (use the crossterm `MouseEvent` type directly).
- `ModelList` populates `thinking_budget` (daemon mode) and the local-mode startup path does too.
- Existing tests updated where `ModelInfo` literals gain the new field.

### B5. TUI verification checkpoint (before any web/desktop work)

- `cargo test -p mew-catalog -p mew-protocol -p mew-tui -p mew`, then `cargo test --all`, `cargo clippy --all -- -D warnings`, `cargo fmt`, `just arch-check`.
- Manual smoke via `mew tui-capture` harness if practical (deterministic local mode) — at minimum the unit tests cover resolution → action → daemon roundtrip paths.

## Phase C: Port to the other frontends

### C1. Web UI (`mew-web-ui/src/components/model-pill.tsx`)

- When `currentModelInfo.thinking_budget` is set, render a "Budget" block under the variant rows: `<input type="range">` (min/max/step) + value label.
- Slider value: current `budget:<n>` variant if active, else the active effort's mapped budget (`by_effort`), else `default`. Local display updates on input; commit via `client.setThinkingVariant("budget:<n>")` on pointer-up / key-up / blur **without closing the popup** (new `handleSetBudget`, unlike `handleSetVariant` which closes).
- Pill badge (`currentThinkingVariant`): strip `budget:` prefix for display.
- `mew-web-client` roundtrip/type tests if the existing test harness covers ModelInfo; otherwise rely on `tsc` + build.

### C2. Desktop GPUI (`apps/mew-desktop`)

- `shell/helpers.rs`: add `thinking_budget_for_model(...) -> Option<mew_protocol::ThinkingBudgetInfo>` mirroring `thinking_variants_for_model`.
- `shell/preferences.rs::render_thinking_picker`: when budget metadata exists, render a custom slider below the options (this GPUI rev has no Slider element; `Div` has `on_drag`/`on_drag_move` + `on_mouse_down`): track + fill + thumb, value from the active variant (same resolution as web), click/drag sets value, commit on release; Left/Right via the existing `shell_key_down` path in `chat.rs` (which currently only handles Escape/copy/platform commands — add arrow handling when the thinking picker is open and the slider has focus).
- `Shell` state: `thinking_budget_draft: Option<i64>` (reset in `lifecycle.rs`/`preferences.rs` close paths, same pattern as `thinking_picker_open`).
- `shell/chat_render.rs`: model label strips `budget:` prefix (format helper).
- Tests in `shell/tests.rs`: budget helper returns metadata only for models that declare it (update existing `ModelInfo` literals with the new field).

## Phase D: Verification

- `cargo test --all`, `cargo clippy --all -- -D warnings`, `cargo fmt`, `just arch-check`, `just theme-codegen-check`.
- `pnpm --filter mew-web-client exec tsc --noEmit`, `pnpm --filter mew-web-ui test`, `pnpm --filter mew-web-ui build`.
- `cargo check -p mew-desktop` (build is heavy; check + targeted tests).
- Update `docs/using-mew/providers.md` if it has a model-config/thinking section (add a short "thinking variants and token budgets" note); `CURRENT.md` dated entry after verification.
