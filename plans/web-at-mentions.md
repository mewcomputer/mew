# Web @-mention parity with the TUI

## Goal

Make the web/desktop composer's `@` references behave like the TUI's: files, skills, models, and subagents can be picked inline; resolution happens once in the daemon; the transcript shows a short notice instead of raw tokens or inlined file bodies.

## Current state (observed)

- TUI parses refs in `crates/mew-tui/src/app/mod.rs` (`parse_namespace_refs`) and resolves them in `crates/mew/src/runtime/mentions.rs`, called from `crates/mew/src/runtime/dispatch.rs` before `CommandTarget::prompt`.
- Resolution produces `(enriched, display, attachments)`: skill bodies and subagent descriptions are appended for the model, model refs become a marker, text files are inlined, image files become `Part::File`, and the visible message gets short `<... added to context>` notices.
- Web composer `mew-web-ui/src/components/input-area.tsx` only opens a persona menu when the whole input starts with `@` and sends the text literally.
- `mew-protocol` `Prompt` / `SubmitPrompt` carry only `text` and `attachments`; there is no wire catalog for skills or subagent definitions. The web store has models and runtime subagent tasks, not catalogs.

## Approach: daemon-side resolution, web-side picker

### Phase 1 — shared resolver

1. Move the namespace parser and `process_mentions` out of the TUI into a shared location usable by the daemon. Prefer an existing crate whose dependencies already include `mew-message`, `mew-skills`, and `mew-subagents`; otherwise add a small `mew-mentions` crate. Do not make the daemon depend on `mew-tui`.
2. Keep the three outputs and their exact current semantics, including silent stripping of unknown skill/subagent names and the file/image split.
3. Keep `mew-tui` tests green by re-exporting or importing the parser from its new home.

### Phase 2 — protocol and daemon

1. Add `#[serde(default)] display_text: Option<String>` to `Prompt` and `SubmitPrompt` in `crates/mew-protocol/src/lib.rs`. Additive fields keep older clients working.
2. Add a candidate request/reply to the protocol: `ListMentionCandidates` returning models, skill names/descriptions, and subagent definition names/descriptions. Use the session's existing catalogs; no new state source.
3. In the daemon prompt/submit path, run the shared resolver with the session cwd and catalogs. Commit `display_text` (or its generated short notices) to the transcript; send only `enriched` text and attachments to the agent.
4. Update TUI dispatch to stop resolving locally once the daemon owns it, so only one source of truth remains. If optimistic local display is kept for responsiveness, reconcile against the daemon's display text on `session-diff`.

### Phase 3 — web composer

1. Replace the persona-only menu in `input-area.tsx` with an inline picker anchored to the active `@token` at the caret. Support `@` alone and `@prefix` for each namespace, with arrow keys, Enter, Escape, and trailing-space insertion.
2. Group candidates as Files, Skills, Models, Subagents. Files come from scoped directory listings relative to session cwd; reuse existing `ListDir`/`dirResults`. Skills, models, and subagents come from the new candidate reply.
3. Keep drag/drop and paste image attachments; file mentions that resolve to images should feed the same attachment path rather than a second model.
4. Store the raw token in the draft and let the daemon resolve it; do not duplicate skill bodies or subagent descriptions in the browser.

### Phase 4 — tests and verification

1. Rust: parser/resolver unit tests (including unknown names and image vs text files), daemon test that a prompt with `@skill:...` produces short display text and enriched model input, protocol roundtrip for the new fields and candidate reply.
2. Web: picker tests for caret detection, namespace filtering, keyboard selection, insertion, and session switching; verify no literal `@skill:` reaches `onSend` as resolved content.
3. Run `cargo test -p mew -p mew-daemon -p mew-protocol`, `just test-js`, `pnpm --filter mew-web-ui exec tsc --noEmit`, `just arch-check`, and a rebuilt `just desktop-electron-build`.
4. Manual: in the rebuilt app, pick a file, skill, model, and subagent from the web composer and confirm the visible message is a short notice while the model receives the full content.

## Notes / risks

- Protocol change is additive but touches all clients; run existing JSON roundtrip tests.
- Resolution currently happens before submission in the TUI; moving it to the daemon changes when `context_files` is updated. Preserve turn-manifest behavior if it depends on inlined files.
- Avoid a second source of truth: the web picker offers candidates only; it must not resolve or inline content itself.
