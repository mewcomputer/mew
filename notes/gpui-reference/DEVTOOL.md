# GPUI Devtool

> **Source note:** This guide was originally written for [tanlethanh/zedra](https://github.com/tanlethanh/zedra) and references zedra-specific code paths. The GPUI concepts generalize, but keep the original project context in mind when reading.

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
