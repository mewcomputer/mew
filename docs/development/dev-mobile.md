---
title: Mobile Core Development
description: Building and testing the mew-mobile-core Rust crate and iOS app.
---

The mobile stack is a Rust crate (`mew-mobile-core`) that owns all
protocol knowledge, exposed to Swift via UniFFI. The iOS app is a thin
SwiftUI layer on top.

## Prerequisites

```sh
# iOS compile targets
rustup target add aarch64-apple-ios aarch64-apple-ios-sim

# xcodegen (for the Xcode project)
brew install xcodegen
```

## Build the mobile core for iOS

```sh
just ios-core
```

This recipe:

1. Builds `mew-mobile-core` for both `aarch64-apple-ios` (device) and
   `aarch64-apple-ios-sim` (simulator) in release mode.
2. Generates Swift bindings via `uniffi-bindgen` from the host dylib.
3. Creates an XCFramework from the two `.a` files with FFI headers.

Output:

- `mew-ios/MewMobileCore/Sources/MewMobileCore/mew_mobile_core.swift` —
  generated Swift bindings
- `mew-ios/MewMobileCore/XCFramework/mew_mobile_core.xcframework` —
  universal binary (gitignored, regenerate with `just ios-core`)

### SwiftPM package

`mew-ios/MewMobileCore/` is a SwiftPM package with two targets:

- **`mew_mobile_coreFFI`** (binary target): the XCFramework with the
  static library + FFI headers (modulemap defines the `mew_mobile_coreFFI`
  C module).
- **`MewMobileCore`** (source target): the generated Swift bindings
  that `import mew_mobile_coreFFI` and expose typed Swift types
  (`MobileCore`, `CoreEvent`, `CoreListener`, etc.).

To verify the package builds:

```sh
cd mew-ios/MewMobileCore
xcodebuild -scheme MewMobileCore \
  -destination 'generic/platform=iOS Simulator' build
```

> **Note**: `swift build` (host) will fail because the XCFramework only
> has iOS slices. Use `xcodebuild` for verification.

## Build the iOS app

```sh
cd mew-ios
xcodebuild -project mew.xcodeproj -scheme mew \
  -destination 'platform=iOS Simulator,name=iPhone 17' build
```

Or open `mew.xcodeproj` in Xcode and build/run from the GUI.

Use the checked-in Xcode project for builds and tests. `project.yml` does
not yet describe its test target or current package pin; regenerating the
project would discard those settings.

### Navigation and daemon settings

The root navigation path owns the selected daemon and chat. Appearance
callbacks fetch catalogs; they do not select another connection. Working
locations persist in phone preferences, while drafts remain in memory keyed
by daemon and session. Session lists validate restored chat identities before
falling back to the corresponding machine's list.

`ReadAutomationSettings` and setter acknowledgments return confirmed values.
Auto-title is connection-local; auto-summary uses one shared watch value and
notifies authenticated websocket clients, including clients without an
attached chat. Mobile caches are cleared on disconnect. Queued settings
mutations expire with the originating connection.

The mobile core tracks actually sent create/attach operations so delayed
history does not complete a new-chat request. Typed `SessionOperationFailed`
replies retain operation and target identity before attachment; legacy error
replies remain available to older clients. Swift handles distinct creation
and attachment events and offers a scoped retry after an opening failure.
Reliable typed failure recovery requires an updated daemon.

`AppNavigationTests` and `AppStoreEventHandlingTests` cover restoration,
cross-daemon ownership, drafts, confirmed settings, invite credentials and
recovery races. Their phone preferences use isolated suites. Simulator
capture fixtures are temporary and are removed after visual verification.

## Crate structure

```
crates/mew-mobile-core/
├── Cargo.toml          uniffi + tokio + iroh deps, crate-type = [lib, staticlib, cdylib]
├── uniffi.toml         cdylib_name = "mew_mobile_core_ffi"
├── src/
│   ├── lib.rs          MobileCore struct, connect_and_run, translate_message
│   ├── codec.rs        Lenient decoder (tolerate unknown ServerMessage variants)
│   ├── events.rs       CoreEvent enum, CoreListener trait, DaemonStatus
│   ├── registry.rs     On-device DaemonRegistry (JSON store, never synced)
│   └── state.rs        SessionState part-assembly, DaemonSnapshot
├── src/bin/
│   └── uniffi-bindgen.rs  Binary entry point for `uniffi-bindgen generate`
└── tests/
    ├── m0_spike.rs      Transport spike: iroh connect → WS upgrade → Ping/Pong → NewSession → Prompt
    └── m1_integration.rs  Full event pipeline: MobileCore.connect() → events through listener
```

## The mobile core

### Phone identity

One iroh `SecretKey` per install, generated on first launch. The Swift
layer persists 32 key bytes in the iOS keychain
(`kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` — non-synchronizable,
the key must not iCloud-sync). The NodeId (public key) is displayed in
settings for pairing.

### Connection model

One iroh connection per daemon, mirroring the web client's one-WS
model:

1. `endpoint.connect(node_id, MEW_ALPN)` → `open_bi()` → WebSocket
   client handshake over the QUIC stream.
2. Send `Ping`, record `Pong { version }` for version-skew warnings.
3. Sessions are switched with `AttachSession` on the same connection.
4. `SessionAlert` arrives on any connection regardless of attachment.
5. Connections to daemons the user isn't looking at are lazy: connect
   on demand, plus an optional "keep connected while foregrounded"
   toggle per daemon.

### Reconnect

Exponential backoff (1s, 2s, 4s… cap 30s) with jitter, reset on
success. After reconnect, re-sends `AttachSession` — the daemon
replays `SessionHistory`, and the core rebuilds session state from
the replay.

### Lenient codec

`mew-protocol`'s serde enums reject unknown variants. A newer daemon
adding a `ServerMessage` variant must not kill the phone's connection.
The core decodes each frame to `serde_json::Value`, reads the `type`
tag, and if full decode fails, logs and drops that frame.

### State assembly

The core ports the web store's part-assembly logic:
`Provider(PartStart/PartDelta/PartEnd/MessageEnd)` → parts → messages,
tool call states, pending permission/ask requests, session usage.
`PartUpdated` is authoritative — when it arrives for a part built from
accumulated deltas, replace the accumulated state wholesale.

### File browser

The core exposes a `list_dir(daemon_id, session_id, path?)` method that
sends `ClientMessage::ListDir`. The daemon responds with
`ServerMessage::DirListing { path, entries: Vec<DirEntry> }`, which the
core translates to `CoreEvent::DirListing`. `DirEntry` is a UniFFI
record with `name`, `is_dir`, and `size`.

The iOS app drives this from a `+` button on the chatbar: tapping a
folder navigates in, tapping a file appends its path to the composer.
Defaults to the session cwd (if known) or the daemon default.

### TextDelta coalescing

UniFFI callbacks cross the ObjC bridge per call. A callback per token
is too expensive. The core batches text deltas on a ~16ms tick before
FFI and emits one event per batch.

## Testing

```sh
# Unit tests (fast, no network)
cargo test -p mew-mobile-core --lib

# Integration tests (real iroh, ~45s each)
cargo test -p mew-mobile-core --features test-harness --test m0_spike -- --nocapture
cargo test -p mew-mobile-core --features test-harness --test m1_integration -- --nocapture
```

The `test-harness` feature flag gates the integration tests, which
spin up a real daemon with a fake provider over real iroh endpoints
(N0 preset, relay-based). The M1 test verifies the full event pipeline:
`MobileCore.connect()` → events arrive through the listener →
`Connected` → `DaemonVersion` → `SessionReloaded` → `TurnEnded` →
`TextDelta` → `snapshot()` has messages.

Integration tests use `#[tokio::test(flavor = "multi_thread")]` because
`MobileCore::connect()` calls `tokio::spawn` for the background
connection task.

## CI

The `ios-ci` job (`.github/workflows/ci.yml`) runs on macOS and does
three things:

1. `cargo check` for both iOS targets (`aarch64-apple-ios` and
   `aarch64-apple-ios-sim`).
2. Regenerates the UniFFI Swift bindings from the current Rust interface.
3. Fails if the committed bindings (`mew_mobile_core.swift` and
   `mew_mobile_coreFFI.h`) differ from the regenerated ones.

This is the guard against bindings drifting behind the Rust core, which
breaks the Xcode build and can crash the app on an FFI mismatch. If the
job fails, run `just ios-core` and commit the regenerated files.

## Adding a new CoreEvent variant

1. Add the variant to `CoreEvent` in `events.rs` (with `#[derive(uniffi::Enum)]`).
2. Handle it in `translate_message()` in `lib.rs` — emit the new event
   from the matching `ServerMessage` arm.
3. Regenerate Swift bindings: `just ios-core` (or `cargo run -p mew-mobile-core --bin uniffi-bindgen -- generate ...`).
4. Handle the new event in `AppStore.handleEvent()` in `mew-ios/mew/AppStore.swift`.
5. Rebuild and test the checked-in Xcode project: `just ios-test`.

## Chat, tools, and review verification

Run the mobile/shared client suites and actual daemon transport tests:

```sh
cargo test -p mew-client-core -p mew-mobile-core -p mew-protocol \
  -p mew-diff -p mew-session --all-targets --features mew-mobile-core/test-harness
cargo test -p mew-daemon --features iroh
cargo clippy -p mew-mobile-core --all-targets --features test-harness -- -D warnings
cargo clippy -p mew-daemon --all-targets --features iroh -- -D warnings
just ios-core
just ios-test
```

These cover attachment and reconnect identities, ordered history pages,
tool-state projection, worker transcript authorization and size limits,
UTF-8 output chunks, and workspace review freshness/errors. The iOS suite
also checks stale reply rejection, output recovery and streaming while
loading older messages, consecutive tool summaries, invisible turn boundaries,
and expanded tool details surviving history-page merges.
Timeline tests cover millisecond timestamps, chronological ordering, calendar
boundaries including daylight saving, daemon-scoped alert promotion, unknown
chat discovery, and refreshes that preserve the active conversation.

Daemon management uses correlated `read_daemon_settings`,
`save_session_defaults`, `reset_session_defaults` and `revoke_device` commands.
Only full-control connections may use them. Defaults persist in
`new-chat-defaults.json` under the session directory; reset restores startup
configuration, and resumed chats keep their own policy. Device snapshots
support legacy trusted nodes and scoped remote pairings. An unavailable device
manager is distinct from an empty device list.

Management replies retain daemon/request identity; unsolicited snapshots do not
complete another pending mutation. The app expires unanswered requests after
five seconds and invalidates them on disconnect. Older-daemon unknown-command
errors and unsent management commands must never clear a chat's pending send
or mark its turn failed. Revocation persists before confirmation, rechecks
control during asynchronous operations and closes active connections with a
bounded WebSocket handshake. The real iroh M0 test covers read/save/reset,
self-revocation confirmation and clean closure.

Physical-phone checks remain necessary before release:

- Pair a fresh identity with an invite, including an expired invite and
  an already consumed invite. Connect to two daemons and switch chats.
- Save and reset new-chat defaults, create chats from two clients and verify
  resumed chats keep their policy. Revoke another device and then this phone;
  check confirmation, disconnection and re-pairing. Check older daemons and
  read-only connections without disrupting an active chat.
- Send and cancel a reply, load older messages during streaming, then
  leave the app and return. Check reading position, keyboard behavior,
  pending permission/question sheets, and reconnect after a network change.
- Search and expand completed, running and failed tools. Check VoiceOver
  labels, large text and reduced motion. Check mixed summaries such as
  "Edited 1 file and ran 2 commands", expand the individual calls, then load
  older history and confirm the open group and output remain visible.
- Open a worker transcript while it runs and keep it open as the worker
  finishes. Read multiple job-output chunks, refresh, and reconnect after
  reading part of the output.
- Review staged, unstaged, renamed, untracked and binary files. Refresh
  after a same-line-count edit and check light/dark and narrow layouts.

For long sessions, measure scrolling, streaming latency and memory on the
phone. Chat retains loaded pages and uses an eager stack to preserve parsed
markdown; simulator renders alone do not establish its performance limit.

## Known issues

- The XCFramework only has `arm64` slices (device + simulator). Intel
  Macs running the simulator need `x86_64-apple-ios-sim` added to the
  build recipe.
- The xcframework headers need `module.modulemap` (not
  `<name>.modulemap`) for SwiftPM binary targets to import correctly.
  The `just ios-core` recipe handles this with a temp headers dir.
- `swift build` on macOS host fails — the xcframework has no macOS
  slice. Use `xcodebuild` for verification.
