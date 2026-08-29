# silent-drop audit: the drain-bug class, repo-wide

Follow-up scan after the runtime dispatch refactor: searched every crate + frontend for the
same defect classes — silent-drop catch-alls on dispatch enums, stub arms with comments
claiming behavior that doesn't exist, duplicated dispatch that drifted, and capability
claims that contradict the code. Severity labels per .mew/agents/code-reviewer.md.

## findings

**[P1] the daemon has slash-dispatch copy #5, and it drops unknown commands silently.**
`ClientMessage::SlashCommand` handling (crates/mew-daemon/src/lib.rs:~730-790) matches
`/clear`, `/compact`, `/wiki`, then `_ => None` — no `SlashResult` is ever sent, so the
client that forwarded the command waits on nothing. Compounding drift: `/wiki` exists
*only* here. It's not in the TUI's builtin list, and the TUI daemon loop forwards only
`/clear`/`/compact` (everything else is handled "locally" where unknown commands die), so
a real daemon feature is unreachable from the TUI entirely.

**[P1] TUI daemon-mode capability alerts are false.**
The daemon implements every `ClientMessage` variant, including `SwitchModel`,
`SetThinkingVariant`, `SetPermissionMode`, and `SwitchPersona` — the web client actively
uses `switch_model`. But `handle_slash_result_local` (crates/mew/src/main.rs:2327-2386)
tells the user "model switching not available in daemon mode", "persona switching not
available in daemon mode", "thinking variant switching not available in daemon mode".
These are stale stubs, not capability facts. Consequence for the runtime rework: the
`DaemonTarget` open question is resolved — implement these against the existing protocol
now; `Unsupported` is only needed for genuinely missing ops (e.g. `rewind`, plugin
commands).

**[P1] daemon slash results render as errors in the TUI.**
crates/mew-daemon/src/client.rs:566-571 maps `ServerMessage::SlashResult` to
`AgentEvent::Error` with the comment "For now, emit as an Error so it's visible. TODO:
emit as a synthetic text AgentEvent." So every successful daemon slash response ("context
cleared", wiki progress) displays through the error path. Textbook forgotten-stub.

**[P1] iOS/mobile never surfaces daemon errors.**
mew-mobile-core's ServerMessage translation ends in a warn-and-ignore catch-all
(crates/mew-mobile-core/src/lib.rs:1454). `ServerMessage::Error` and `ErrorEvent` are not
translated to any `CoreEvent`, so a daemon-side failure ("no session", agent error) is a
log line on the phone and a silently stalled turn in the UI. Also untranslated:
`ToolStart`/`ToolEnd`/`ToolProgress`, `Subagent*`, `JobUpdate`, `SessionSummaryChanged` —
some plausibly deliberate mobile scope, but `Error`/`ErrorEvent` are not a scoping
decision. (Credit where due: the catch-all at least logs, unlike the TUI drain did.)

**[P2] web client's message switch has no exhaustiveness guard.**
mew-web-client/src/index.ts:996-1227 switches over the typed `ServerMessage` union with no
`default`. Ignoring unknown *wire* frames at runtime is correct forward-compat; the gap is
compile-time: adding a variant to the TS union (or the generator that produces it)
compiles cleanly while unhandled. One `default: { const _x: never = msg; }` arm turns new
variants into type errors — the TS equivalent of `deny(wildcard_enum_match_arm)`.

**[P2] two more forgotten "for now" stubs** worth tickets, not this effort:
crates/mew-daemon/src/iroh_transport.rs:205 ("TODO: hook this in properly"),
crates/mew-hooks-runtime/src/runtime.rs:273-277 (plugin restart slot refactor).

## reviewed and cleared (so nobody re-audits them)

- provider-event `_ => {}` filters in daemon title/summary generation
  (lib.rs:1435, 1698) — they intentionally want only text deltas.
- `apply_delta` field-name filters (mew-agent/src/events.rs:139-141) — unknown *fields*
  on a known part, correct to ignore.
- web-bridge HTTP header parsing catch-all (main.rs:112) — non-dispatch.
- `append_to_part` (mew-tui/src/app.rs:3047) — deltas only apply to text/reasoning parts.
- mobile lenient decode dropping unknown wire frames (spec note #7) — deliberate
  forward-compat, and distinct from the typed-enum catch-all flagged above.
- daemon `ClientMessage` dispatch — every protocol variant handled, no catch-all.
- harness `apply_action` stub — already slated (runtime plan stage 4).

## remediation

1. **fold into the runtime rework (stages 1-2):** finding 2 changes `DaemonTarget` from
   "alerts first, protocol later" to "wire up now". Finding 3 is a one-arm fix in
   client.rs (`SlashResult` → synthetic text event) — do it in stage 2 when the daemon
   path is touched anyway.
2. **shared command registry (new, small):** the root cause of finding 1 + `/wiki`
   invisibility is that "what commands exist and where they execute" lives in five
   uncoordinated lists. Make it data: one table (name, description, locus:
   client|daemon|either) in mew-protocol or a tiny shared crate. TUI autocomplete,
   `handle_slash`, the daemon dispatcher, and the daemon's unknown-command reply
   (`SlashResult: "unknown command /x"` — never silence) all derive from it. Slot after
   runtime stage 2.
3. **mobile-core:** replace the warn catch-all with explicit grouped arms (client.rs
   style) so new protocol variants force a decision at compile time; translate `Error`/
   `ErrorEvent` to a user-visible `CoreEvent` (alert or synthetic message — whatever the
   iOS error surface is). Independent of the runtime rework; pairs with the ios plan work.
4. **web client:** add the `never`-default to the switch. One-liner plus a lint note in
   the package.
5. **arch-check extension (runtime stage 4):** the grep ratchet should also fail on new
   `SlashCommand`-string matches outside the shared registry once (2) lands.
