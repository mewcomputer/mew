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
xcodegen generate    # creates mew.xcodeproj from project.yml
xcodebuild -project mew.xcodeproj -scheme mew \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro' build
```

Or open `mew.xcodeproj` in Xcode and build/run from the GUI.

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

## Adding a new CoreEvent variant

1. Add the variant to `CoreEvent` in `events.rs` (with `#[derive(uniffi::Enum)]`).
2. Handle it in `translate_message()` in `lib.rs` — emit the new event
   from the matching `ServerMessage` arm.
3. Regenerate Swift bindings: `just ios-core` (or `cargo run -p mew-mobile-core --bin uniffi-bindgen -- generate ...`).
4. Handle the new event in `AppStore.handleEvent()` in `mew-ios/mew/AppStore.swift`.
5. Rebuild the app: `cd mew-ios && xcodegen generate && xcodebuild ...`.

## Known issues

- The XCFramework only has `arm64` slices (device + simulator). Intel
  Macs running the simulator need `x86_64-apple-ios-sim` added to the
  build recipe.
- The xcframework headers need `module.modulemap` (not
  `<name>.modulemap`) for SwiftPM binary targets to import correctly.
  The `just ios-core` recipe handles this with a temp headers dir.
- `swift build` on macOS host fails — the xcframework has no macOS
  slice. Use `xcodebuild` for verification.
