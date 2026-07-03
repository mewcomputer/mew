# mew ios app — spec

An iOS client for mew: connect to daemons on multiple machines over iroh,
watch sessions, respond to permissions, send prompts. Companion to
`notes/mew-mobile-iroh-plan.md` (POC ladder rungs 3–4) and forward-compatible
with `notes/mew-accounts-roaming-plan.md` (the phone's iroh key becomes its
device key when the hub exists).

Status: spec. Nothing mobile-side exists yet. Daemon-side iroh stage 1
(listener + allowlist + `mew pair`) is implemented in the working tree but
not merged — treat it as a prerequisite that lands first, and expect its
details (pairing output format, allowlist path) to still shift.

---

## Goals / non-goals

**v1 goals**

- pair with any number of daemons, each on a different machine
- per-daemon session rail with needs-you ordering, running state, usage
- attach to a session: full history replay + live streaming
- send prompts, cancel turns, answer permission and ask-user requests
- switch models, set permission mode, rename/archive/pin sessions
- local notifications for `SessionAlert` while the app is foregrounded
- graceful reconnect when the app returns to foreground

**explicitly out of scope for v1**

- push notifications while backgrounded (needs the push relay — v2, per the
  mobile plan)
- file browser / previews / git status (protocol supports it; defer the UI)
- session groups management (render group labels read-only if cheap; no CRUD)
- prompt attachments (the wire `Attachment` is a daemon-side path; sending a
  photo from the phone has no protocol support yet)
- plugins / custom UI panels, personas UI, subagent drill-in beyond status
- hub, accounts, vault — everything in the accounts plan. this app must work
  with zero mew infrastructure, same as the daemon.

---

## Architecture

three layers, two languages:

```
┌────────────────────────────────────────────┐
│ SwiftUI app (mew-ios)                      │
│   daemon list · session rail · chat ·      │
│   permission/ask sheets · settings         │
├────────────────────────────────────────────┤
│ mew-mobile-core (Rust, UniFFI)             │
│   iroh endpoint + phone identity           │
│   per-daemon connections + reconnect       │
│   WS framing + mew-protocol codec          │
│   session state assembly (parts→messages)  │
├────────────────────────────────────────────┤
│ iroh (QUIC, holepunching, relays)          │
└────────────────────────────────────────────┘
```

the rule: **all protocol knowledge lives in Rust.** Swift never sees
`ClientMessage`/`ServerMessage` JSON or `ProviderEventWire` deltas; it sees
a typed, app-shaped API. this is the same layering the web UI has
(`mew-web-client` + Zustand store), collapsed into one crate so it isn't
reimplemented per platform.

### mew-mobile-core

new workspace crate, `crates/mew-mobile-core`. depends on `mew-protocol`,
`mew-message`, `iroh`, `tokio`, `tokio-tungstenite`, `uniffi`. no default
features on anything heavy; this crate never appears in the daemon or TUI
dependency graph.

**phone identity.** one iroh `SecretKey` per install, generated on first
launch. the Swift layer persists the 32 key bytes in the iOS keychain
(`kSecAttrAccessibleAfterFirstUnlock`, non-synchronizable — the key must not
iCloud-sync to another device, that would violate "device private keys never
leave devices" from the accounts plan) and hands them to the core at init.
NodeId (the pubkey) is displayed in settings for pairing. when the hub
exists later, this same key enrolls as a `Device { kind: Client }`.

**connection model.** one iroh connection per daemon, mirroring the web
client's one-websocket model:

- `endpoint.connect(node_id, MEW_ALPN)` → `open_bi()` → **WebSocket client
  handshake over the QUIC stream** (`tokio_tungstenite::client_async` with a
  placeholder URL, e.g. `ws://daemon.mew/`). stage 1 daemon wraps the stream
  in `accept_async`, so the client must speak the WS upgrade. yes, this is
  double-framed; it matches the daemon and gets deleted together with the
  daemon's WS layer when framing is cleaned up post-stage-1.
- immediately send `Ping`, record `Pong { version }` for the daemon registry
  and version-skew warnings.
- sessions are switched with `AttachSession` on the same connection, exactly
  like the web UI. one attached session per daemon connection.
- `SessionAlert` and `SessionAttentionChanged` arrive on any connection
  regardless of attachment, so a connected-but-idle daemon still surfaces
  alerts.
- connections to daemons the user isn't looking at are lazy: connect on
  demand (daemon opened in UI) plus an optional "keep connected while
  foregrounded" toggle per daemon. every open connection costs a QUIC
  keepalive; on a phone radio that's not free.

**reconnect.** exponential backoff (1s, 2s, 4s… cap 30s) with jitter,
reset on success. reconnect triggers: stream error, app returning to
foreground, network path change (iroh handles migration for live
connections; the core only needs to redial dead ones). after reconnect,
re-send `AttachSession` — the daemon replays `SessionHistory`, which is the
state-recovery mechanism; the core rebuilds session state from the replay
and emits a single coarse `SessionReloaded` event so the UI swaps state
without a delta storm.

**decode leniency.** `mew-protocol`'s serde enums reject unknown variants.
a newer daemon adding a `ServerMessage` variant must not kill the phone's
connection: the core first decodes each frame to `serde_json::Value`, reads
`type`, and if full decode fails, logs and drops that frame. version skew is
already surfaced via Ping/Pong; a 90-day interop policy is specced in the
accounts plan and this is the client half of honoring it.

**state assembly.** the core ports the web store's part-assembly logic:
`Provider(PartStart/PartDelta/PartEnd/MessageEnd)` → parts → messages, tool
call states, pending permission/ask requests, todos, session usage. Swift
receives granular events for the hot path (text deltas) and holds a mirror
of the coarse state. after reconnect or attach, Swift pulls a full
`snapshot()` instead of replaying deltas.

### UniFFI surface (approximate)

```rust
// object: one per app launch
interface MobileCore {
    constructor(secret_key: Vec<u8>, data_dir: String);
    fn node_id(&self) -> String;

    fn add_daemon(&self, node_id: String, name: String) -> DaemonId;
    fn remove_daemon(&self, id: DaemonId);
    fn list_daemons(&self) -> Vec<DaemonInfo>;   // name, node_id, status, version

    fn connect(&self, id: DaemonId);             // async, events report progress
    fn disconnect(&self, id: DaemonId);

    fn list_sessions(&self, id: DaemonId);       // → SessionList event
    fn new_session(&self, id: DaemonId, cwd: Option<String>);
    fn attach(&self, id: DaemonId, session_id: String);
    fn prompt(&self, id: DaemonId, text: String);
    fn cancel(&self, id: DaemonId);
    fn respond_permission(&self, id: DaemonId, request_id: u64, decision: Decision);
    fn respond_ask_user(&self, id: DaemonId, request_id: u64, answers: Vec<String>);
    fn list_models(&self, id: DaemonId);
    fn switch_model(&self, id: DaemonId, provider: String, model: String);
    fn set_permission_mode(&self, id: DaemonId, mode: String);
    fn rename_session(&self, id: DaemonId, session_id: String, title: String);
    fn archive_session(&self, id: DaemonId, session_id: String, archived: bool);
    fn pin_session(&self, id: DaemonId, session_id: String, pinned: bool);
    fn delete_session(&self, id: DaemonId, session_id: String);

    fn snapshot(&self, id: DaemonId) -> DaemonSnapshot;  // full mirror state
    fn set_listener(&self, listener: Box<dyn CoreListener>);
}

// callback interface implemented in Swift
trait CoreListener {
    fn on_event(&self, ev: CoreEvent);
}

enum CoreEvent {
    DaemonStatusChanged { daemon, status },        // connecting/connected/backoff/paired-lost
    SessionList { daemon, sessions },
    SessionReloaded { daemon, session_id },        // pull snapshot
    TextDelta { daemon, session_id, part_id, delta },
    PartUpdated { daemon, session_id, part },      // tool state, reasoning, errors
    TurnEnded { daemon, session_id, usage, failed },
    PermissionRequested { daemon, session_id, request },
    AskUserRequested { daemon, session_id, request },
    RequestResolved { daemon, request_id },        // dismiss sheet, any device answered
    Alert { daemon, session_id, kind, title, detail },
    AttentionChanged { daemon, session_id, pending_permissions, pending_questions },
    TodosUpdated { daemon, session_id, todos },
    ModelList { daemon, models },
    SlashResult { daemon, session_id, text },
}
```

exact shapes to be settled during implementation; the invariant is that
`CoreEvent` is app-vocabulary (sessions, turns, requests), not
wire-vocabulary.

### daemon registry (on-phone)

small JSON (or SwiftData) store, local only, never synced:

```
DaemonEntry { node_id, name, added_at, last_connected_at,
              last_known_version, keep_connected: bool }
```

when the hub exists, this registry becomes a cache of the account's daemon
list instead of the source of truth. nothing in v1 should assume it's
hand-curated forever — keep it behind one Swift type.

---

## Pairing UX

matches daemon stage 1 (`mew pair` auto-approves the first connection):

1. user runs `mew pair` on the daemon machine; it prints the NodeId (QR
   printing is deferred daemon-side, so text is the baseline).
2. phone: "Add daemon" → paste NodeId **or** scan QR. accepted formats:
   - a raw NodeId (z-base-32 / hex, whatever iroh prints)
   - a `mew001:` payload per the accounts plan's reserved prefix, carrying
     `{ node_id, name? }`. adopt this now so the daemon's future QR output
     and this parser meet in the middle. parser rejects unknown versions
     loudly.
3. phone names the daemon (prefilled from payload name or hostname if
   present), core dials it. `mew pair` adds the phone's NodeId to the
   allowlist, closes with "pairing complete", and the daemon proper (with
   `--iroh`) accepts the phone thereafter.
4. failure modes surfaced distinctly: pairing window timed out (120s),
   unauthorized (paired with a different daemon identity than expected),
   unreachable (relay timeout).

note: stage 1 pairing closes the connection after allowlisting, so the app
should expect connect→close→reconnect during pairing, and treat the *second*
successful connection as "paired and live".

---

## App structure (SwiftUI)

navigation: `Daemons → Sessions → Chat`, with sheets for interrupts.

- **daemon list.** each row: name, status dot (connected / connecting /
  unreachable / backoff), daemon version, aggregate needs-you badge summed
  from that daemon's sessions. pull-to-refresh redials.
- **session rail (per daemon).** ports the web rail's ordering: needs
  attention (amber, pending permissions/questions) → running → active →
  idle; archived behind a filter. row shows title (`display_title`
  precedence: custom > summary > id), state, cost badge from
  `SessionUsageWire`, change stats if present. swipe actions: pin, archive,
  rename, delete (confirm). "+" creates a session (cwd defaults to daemon
  cwd; free-text cwd field behind an advanced disclosure).
- **chat view.** history from replay + live streaming. renders text parts
  as markdown (static parts cached; the streaming tail re-rendered per
  delta batch — coalesce deltas to display-link cadence, don't relayout per
  token). reasoning parts collapsed by default. tool calls as compact rows:
  name, state (pending/running/completed/error), expandable input/output.
  subagent status lines with the `↳ last progress` pattern from the TUI
  sidebar. sticky composer with send/cancel (cancel replaces send while a
  turn runs). model + permission-mode pickers in the toolbar. slash
  commands: plain text passthrough via `SlashCommand` for daemon-side ones;
  no client-side command palette in v1.
- **permission sheet.** tool name, pretty-printed input (bash command
  front-and-center), allow once / allow session / deny. also covers
  workspace-permission and subagent-permission requests (label the
  subagent). dismissed automatically on `RequestResolved` (another device
  answered).
- **ask-user sheet.** one to four questions, options as buttons +
  free-text field.
- **settings.** this phone's NodeId (copyable/QR), daemon management
  (rename/remove/keep-connected), notification preferences, version.

**alerts.** while foregrounded, `SessionAlert` for a session other than the
one on screen → local notification (`UNUserNotificationCenter`, immediate
trigger) + in-app banner; tapping navigates to that daemon/session. badge
count = total pending permissions + questions across daemons. all cleared
on attach, mirroring the web UI's `clearAlertsForSession`.

**lifecycle.** on `scenePhase == .background`, expect iOS to kill QUIC in
~30s: mark connections suspect, don't fight it. on foreground: redial,
re-attach, snapshot-swap. an optional `BGAppRefreshTask` may opportunistically
poll for pending permissions a few times an hour, but it is best-effort and
must not be presented to the user as push.

---

## Protocol subset (v1 contract)

client → daemon: `Ping`, `NewSession`, `AttachSession`, `ListSessions`,
`Prompt`, `Cancel`, `PermissionResponse`, `AskUserResponse`, `SlashCommand`,
`ListModels`, `SwitchModel`, `SetPermissionMode`, `RenameSession`,
`ArchiveSession`, `PinSession`, `DeleteSession`.

daemon → client, must handle: `Pong`, `SessionReady`, `SessionList`,
`SessionHistory`, `Provider(*)`, `UserMessage` (dedupe own prompts by text,
same as web), `ToolStart/ToolEnd/ToolProgress`, `PartUpdated`,
`PermissionRequest`, `WorkspacePermissionRequest`, `SubagentPermissionRequest`,
`AskUserRequest`, `RequestResolved`, `SessionAlert`,
`SessionAttentionChanged`, `SessionActivityChanged`, `SessionUsageChanged`,
`SessionTitleChanged`, `SessionSummaryChanged`, `SessionMetaChanged`,
`SessionCleared`, `ModelList`, `ModelSwitched`, `PermissionModeChanged`,
`SubagentStart/Status/End`, `TodosUpdated`, `SlashResult`, `Error`,
`ErrorEvent`.

tolerate-and-drop: everything else (`ClientKind` should be extended with a
`Mobile` variant — one-line protocol change, additive, do it while stage 1
is still unmerged).

new `ClientKind::Mobile` is the **only** protocol change this app needs.

---

## Packaging & build

- `mew-mobile-core` compiled for `aarch64-apple-ios` +
  `aarch64-apple-ios-sim`, bundled as an XCFramework; UniFFI-generated Swift
  bindings wrapped in a local SwiftPM package (`MewMobileCore`).
- a `just` recipe (`just ios-core`) drives cargo build + `uniffi-bindgen` +
  `xcodebuild -create-xcframework` so the Xcode project consumes one
  artifact.
- app code lives in `mew-ios/` at the repo root (sibling of `mew-web-client`,
  not in the cargo workspace).
- distribution: personal team sideload / TestFlight. App Store is a later
  question, not a v1 concern.

---

## Milestones

each independently useful; stop-anywhere, same as the POC ladder.

- **m0 — cross-compile spike (de-risk first).** empty crate depending on
  iroh + tokio, built for the two iOS targets, dialed from a unit-test-shaped
  harness on the simulator against a local daemon: connect, WS upgrade,
  Ping/Pong. this answers the two scary unknowns — iroh's crypto backend on
  iOS and the WS-client-over-QUIC handshake — before any real code exists.
  (equivalent to POC ladder rung 2, with the phone as the second peer.)
- **m1 — mew-mobile-core.** identity, registry, connect/reconnect, codec,
  state assembly, full `CoreEvent` surface. tested against the daemon with
  `mew-provider-fake` over real iroh, following the existing
  `crates/mew-daemon/tests/iroh.rs` pattern.
- **m2 — bindings + package.** UniFFI, XCFramework, SwiftPM package, a
  Swift XCTest that pairs and round-trips a prompt on the simulator.
- **m3 — app v1.** pairing flow, daemon list, session rail, chat with
  streaming, permission/ask sheets, cancel, model picker.
- **m4 — polish.** alerts + local notifications, needs-you ordering and
  badges, reconnect UX, settings.

---

## Test plan

- **core (Rust, most of the coverage lives here):** unit tests for codec
  leniency (unknown `type` dropped, connection survives), state assembly
  (delta streams → messages, tool states, request lifecycles), backoff
  logic. integration tests against a real daemon + fake provider over iroh:
  attach/replay, prompt/stream, permission round-trip including
  `RequestResolved` from a second client, reconnect-and-replay.
- **swift:** thin — view-model tests against a scripted `CoreListener`
  event sequence; one end-to-end simulator test (m2).
- expected errors asserted explicitly (unauthorized peer, pairing timeout,
  version-skew warning path).

---

## Risks

- **iroh on iOS.** iroh 1.0 claims broad platform support and iroh-ffi ships
  SwiftPM artifacts, so the path exists, but our own crate cross-compiling
  cleanly (crypto backend, no `fork`, bitcode-free) is unverified. that's
  why m0 is first and tiny.
- **double framing.** WS-client-handshake-over-QUIC is unusual;
  `client_async` over a custom stream should just work, but it's exercised
  nowhere else in the codebase yet. also m0.
- **strict serde.** without decode leniency, every additive daemon change
  bricks older phones. the leniency layer is non-optional, and worth
  considering upstream in `mew-web-client` too.
- **stage 1 is unmerged.** pairing output format, allowlist path, ALPN are
  all still movable. the spec intentionally binds to the ALPN
  (`mew/wire/0`) and the message schema, and keeps pairing parsing behind
  one function.
- **foreground-only is a real UX ceiling.** the headline mobile use case
  ("phone buzzes when the agent needs me") does not work until the v2 push
  relay. set expectations in the app itself (a one-time explainer), or v1
  reads as broken.

## Notes for mew-mobile-core implementers

specific to the stage 1 code as it sits in the working tree (2026-07-02),
ordered by severity.

1. **the daemon has no stable NodeId yet — this blocks the registry model.**
   neither `run_iroh` nor `pair_cmd` passes a secret key to
   `Endpoint::builder`, so both get a fresh random keypair per run. two
   consequences: the daemon's NodeId changes on every restart (the phone's
   daemon registry keys on NodeId, so entries die on restart), and worse,
   `mew pair` binds its **own** endpoint — the NodeId the phone pairs
   against is not the daemon's NodeId, and the phone never learns the real
   one. end-to-end, stage 1 pairing cannot currently produce a dialable
   daemon entry. fix is daemon-side (persist a secret key sidecar, e.g.
   next to `authorized_nodes.json`, and have `mew pair` load the same key
   or run inside the daemon process), but core work should not start
   integration testing against `mew pair` until it lands.
2. **pairing success is signaled by a connection close, not a handshake.**
   `pair_cmd` allowlists the peer and immediately closes with application
   code 0 / reason `pairing complete` (rejection: code 1 / `unauthorized`
   from the daemon proper). during pairing the core must *not* attempt the
   WS upgrade — expect connect → close, read the close code/reason to
   distinguish outcomes, and treat the next successful connection (to the
   daemon, once note 1 is fixed) as "paired and live". keep this in one
   function; it's the most likely thing to change post-stage-1.
3. **`open_bi` is lazy — write first.** the daemon's `accept_bi()` only
   fires once the client sends bytes on the stream. `client_async`'s WS
   handshake writes immediately so the happy path works, but any "open the
   stream, then wait for X" sequencing on the client will deadlock. open,
   then start the handshake, unconditionally.
4. **one bi stream per connection.** `MewIrohHandler::accept` calls
   `accept_bi` exactly once. if the stream errors while the QUIC connection
   survives, do not open a second stream — redial the connection.
5. **don't send `client_kind: "mobile"` until the variant exists
   daemon-side.** serde rejects unknown enum values, and it fails the
   *entire* `NewSession`/`AttachSession` message, not just the field. until
   `ClientKind::Mobile` is merged and confirmed via the `Pong` version,
   omit the field (defaults to `unknown`).
6. **namespace `request_id` by daemon.** it's a per-daemon counter; with
   multiple connected daemons the u64s will collide. every pending-request
   map in the core keys on `(daemon_id, request_id)`, and `RequestResolved`
   resolves within its daemon only.
7. **build the lenient codec first, not last.** every other feature funnels
   through it, and retrofitting leniency after direct
   `decode_json::<ServerMessage>` calls spread is miserable. decode to
   `serde_json::Value`, then to the typed enum; on typed failure, log the
   `type` tag and drop the frame.
8. **coalesce `TextDelta` in Rust, before the FFI.** a UniFFI callback per
   token is an objc-bridge round-trip per token. batch deltas in the core
   (flush on ~16–33ms tick or on part end) and emit one event per batch;
   don't leave coalescing to SwiftUI.
9. **dedupe `UserMessage` by text.** the daemon broadcasts the prompt back
   to all attached clients including the sender; match the web client's
   behavior (drop the first `UserMessage` equal to the last sent prompt).
10. **`PartUpdated` is authoritative.** when it arrives for a part built
    from accumulated deltas, replace the accumulated state wholesale. the
    web UI's store is the reference implementation for all of the
    assembly logic — port its semantics, not just its shape.
11. **NodeId-only dialing depends on public discovery being on.** it works
    today because both sides use the `N0` preset (pkarr discovery +
    relays). the accounts plan disables public discovery once the hub is
    the resolver, at which point the pairing payload must carry relay/addr
    info. keep dial-info parsing behind the same single function as note 2.

## Open questions

1. does `ClientKind::Mobile` land with stage 1 or as a follow-up? (additive
   either way; cheaper before the TS client re-exports the union.)
2. session history for long sessions: replay is full-history today. fine
   for v1? if phones choke, pagination is a daemon-side change
   (`AttachSession { since }`-shaped) — flag early if needed.
3. markdown rendering: swift-markdown-ui dependency vs `AttributedString`
   basic markdown. lean basic-first, upgrade if it looks bad in practice.
4. should the core crate live in the cargo workspace (shared lint/CI) even
   though its targets differ? lean yes, with CI building it for iOS targets
   only on a dedicated job.
