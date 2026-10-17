# mew GPUI verification and tooling

The [pinned source policy](README.md#source-policy) applies to test, inspector and
profiler APIs as well as production views.

## Current checks

Start with the affected desktop tests, then the desktop suite and strict lint:

```sh
cargo test -p mew-desktop
cargo clippy -p mew-desktop --all-targets -- -D warnings
cargo build -p mew-desktop
cargo fmt --all -- --check
just arch-check
just theme-codegen-check
```

Use the matching CEF SDK via `CEF_PATH` when already installed, for example the
local `target/cef-sdk` cache. Metal compilation needs a writable compiler cache.
Set `MEW_CONFIG_DIR`, `MEW_DATA_DIR`, `MEW_CACHE_DIR` and `MEW_SESSION_DIR` to
isolated temporary directories for native tests that create a shell or daemon.
Do not use real settings or sessions as a test fixture.

For entity notification, focus, layout and task lifetimes, use `#[gpui::test]`
with `TestAppContext` and the actual entity/view. The desktop dev-dependency
enables `gpui/test-support`; production retains `gpui/profiler`. Drive GPUI timers
through its test dispatcher. A pure range/state helper test does not establish
paint, invalidation or transport behavior.

Text shaping uses `window.text_system()` (`WindowTextSystem`), rather than the
app's shared `TextSystem`. In a GPUI test, obtain it through an empty test window
with `cx.add_empty_window().update(...)`.

Use real localhost transport and subprocess tests for client/daemon boundaries.
Run `just ci` when shared dependencies or runtime behavior warrant the broader
gate. It includes the separate subprocess path and the documentation build.

## Native smoke and frame measurements

Build the native desktop and package it with `scripts/package-desktop-native.sh`
for CEF coverage; `just desktop-dev` also uses the packaged development app.
The package needs the matching CEF framework and helper apps, plus its daemon
and standalone browser tool. A bare executable does not exercise packaging.
Check text entry/IME, keyboard focus, Unicode selection/copy, sidebar/workbench
transitions, resize, light/dark appearance, idle terminal repaint, browser failure
feedback, retry and quit. Run against an isolated real daemon for connection
behavior; the fake-provider option can keep model turns deterministic.

Use a separate bundle identity for a disposable QA app, including its helper
apps, and launch it with the isolated environment before attaching UI tools.
Attaching to a stopped app may launch it without those environment variables.

Check Pinyin candidates with physical keyboard input on the first and wrapped or
second lines, then resize. For CEF, first focus the URL field and then an actual
page input. Copy/paste, selection and command deletion must edit the page while
leaving the URL draft unchanged. Check the native Edit menu too. Background UI
automation can route key equivalents differently or inject control characters;
confirm suspected native keyboard failures with the app in the foreground.

The opt-in real AppKit responder regression runs without writing the clipboard:

```sh
MEW_NATIVE_APPKIT_TESTS=1 cargo test -p mew-cef-host --test native_key_equivalents
```

See the [CEF host guide](../../native/cef-host/README.md) for SDK requirements.

`MEW_DESKTOP_FRAME_TRACE=1` enables the existing frame probe and writes 60-frame
reports to `mew-desktop-frame-trace.log` in the process temporary directory.
At this pin, enable collection with `gpui::profiler::set_trace_enabled(true)`.
`FrameTimingCollector::collect_unseen()` returns `FrameEvent::Draw` and
`FrameEvent::Present`; the desktop draw report counts only draw events and keeps
its cursor across polls. Compare the same interaction and build profile before
and after a performance change. Debug timings during compilation are not a
release performance benchmark.

## Historical Zedra HTTP devtool

The remainder is retained from [Zedra](https://github.com/tanlethanh/zedra).
Its HTTP endpoints, synthetic tap server and `gpui/src/devtool.rs` are fork
facilities, absent from our pinned GPUI and mew. They are historical design
material, not runnable mew instructions. Use the current checks above.

An in-app HTTP server that exposes the latest frame's interactive hitboxes by
`ElementId` path and accepts synthetic taps. Lets an AI agent or test harness
drive the UI without manual reproduction.

## Concept

The devtool runs as a debug-only HTTP server inside the GPUI app. It publishes
the element tree's interactive hitboxes after each frame render, and accepts
synthetic tap requests that get queued onto the main thread's frame loop.

This pattern is portable to any GPUI app — you need:

1. A snapshot registry that collects hitboxes during paint (GPUI's
   `insert_hitbox` / `insert_inspector_hitbox` already provides the data).
2. A small HTTP server that exposes the snapshot as JSON.
3. A tap queue that injects synthetic pointer events on the next frame tick.

## HTTP Surface

All endpoints accept and return JSON.

### `GET /ping`

```json
{"ok": true}
```

### `GET /elements`

```json
{
  "frame_id": 42,
  "entries": [
    {
      "path": "view#4294967297/.../drawer-host/drawer-toggle-btn",
      "instance": 0,
      "x": 0.00, "y": 36.73,
      "w": 41.82, "h": 41.82
    }
  ]
}
```

- `path` — slash-joined `ElementId` chain from root. Developer-tagged
  elements appear with their `.id("…")` string; untagged ancestors render as
  `view#<entity_id>`.
- `instance` — disambiguates same-path elements in one frame.
- `x`/`y`/`w`/`h` — logical-pixel bounds (post scale factor).
- `frame_id` — monotonic, useful for "tap then re-query" synchronisation.

### `POST /tap`

```json
{"element_id": "drawer-toggle-btn"}
```

Accepts either the bare leaf or the full slash path. If a leaf matches
multiple entries, the smallest-area (topmost) entry wins. Returns
`{"ok":true,"x":…,"y":…,"frame_id":…}` or
`{"ok":false,"error":"element not found"}`.

### `POST /tap_xy`

```json
{"x": 540, "y": 120}
```

Coordinates are logical pixels. Returns `{"ok":true,"x":…,"y":…}` or
`{"ok":false,"error":"x and y required"}`.

## Element Coverage

Only GPUI `div` elements (and other `Interactive` callers of
`insert_hitbox`) that have an `ElementId` show up. Tag interactive surfaces
with `.id("my-button")`, `.id("card-0")`, etc. Untagged regions fall through
to `tap-xy`.

## Typical Agent / Test Loop

1. Build + launch the app with devtool enabled (debug builds only).
2. `GET /ping` to confirm liveness.
3. `GET /elements` to discover interactive targets.
4. `POST /tap` with an element ID to drive the UI.
5. Re-query `/elements` to verify state changed (use `frame_id` for sync).

## Design Notes

- Single window assumed. The first registered window receives all taps.
- Taps fire on the next frame tick, not immediately.
- One request per connection — no keep-alive, no concurrency.
- Bind to `127.0.0.1` only; no auth. For remote devices, use port forwarding.
- Release builds compile clean but the server should not start (gate behind
  `debug_assertions` or a `feature = "devtool"`).

## Implementation Locations (in Zed's GPUI)

- `crates/gpui/src/devtool.rs` — element snapshot registry.
- `crates/gpui/src/window.rs` — snapshot publish + relaxed picking-mode guard.
- Platform-specific HTTP server + tap queue (the zedra implementation lives
  in `gpui_android/src/android/devtool_server.rs`).

The gpui-side registry is platform-neutral, so adding a devtool server for
any platform is a matter of mirroring the HTTP loop and tap queue against
that platform's GPUI backend.
