# GPUI Code Conventions

> **Source note:** This guide was originally written for [tanlethanh/zedra](https://github.com/tanlethanh/zedra) and references zedra-specific code paths. The GPUI concepts generalize, but keep the original project context in mind when reading.

## Rust Style

Prioritize correctness and clarity. Treat speed and cleverness as secondary
unless performance is the stated problem.

- Prefer adding behavior to existing files unless the change introduces a real new logical component.
- Avoid creating `mod.rs` module paths. Prefer `src/name.rs`.
- Use full words for variable names. Avoid terse abbreviations like `q` for `queue`.
- Keep comments focused on non-obvious reasons, invariants, lifecycle constraints, or regression guards. Do not add comments that only summarize nearby code.
- Use variable shadowing to scope clones for async moves, so borrowed references do not live longer than necessary.

## Error Handling

Avoid panic-prone shortcuts in normal code paths.

- Prefer `?`, explicit `match`, or `if let Err(err)` over `unwrap()` and `expect()`.
- Be careful with indexing. Prefer checked access when out-of-bounds input is possible.
- Do not silently discard fallible results with `let _ = ...` when the error affects behavior or observability.
- When an error is intentionally ignored, make the reason visible with explicit handling such as `.log_err()` where available, a `tracing` warning, or a short comment for expected shutdown paths.
- Async operations that can fail should propagate errors to the layer that can show useful feedback to the user.

## Imports

Use glob imports for common framework crates:

```rust
use gpui::*;
use tracing::*;
```

Prefer short module paths over inline `crate::` paths:

```rust
use crate::platform_bridge;

let inset = platform_bridge::status_bar_inset();
```

For items used directly, import the item:

```rust
use crate::editor::git_diff_view::{FileDiff, parse_unified_diff};
```

## Logging

Use `tracing::` everywhere. Never `log::` directly.

```rust
use tracing::*;

info!(endpoint = %addr.id.fmt_short(), "session: connecting");
warn!(id = %terminal_id, err = %e, "terminal: attach failed");
```

**Levels**: `error` = broken, `warn` = degraded, `info` = lifecycle events, `debug` = bookkeeping. No `trace`.

**Format**: `"component: verb noun"`, lowercase, no trailing period. Use structured fields for key=value, `{}` (Display) for errors.

## Documentation Style

Write docs for developers who are scanning while trying to do a task.

- Start with the goal or action, not background.
- Put the common path first and edge cases later.
- Use short, direct sentences in present tense.
- Address the reader as `you` in user-facing docs.
- Avoid promotional phrasing, superlatives, filler words, and hedging such as "simply", "just", "easily", "powerful", or "seamless".
- State limitations directly and pair them with a workaround or next step when useful.
- Use `sh` fences for terminal command blocks and backticks for inline commands, paths, settings, and keybindings.
- Show complete working examples, not fragments.

## GPUI Entities And Tasks

- Use the `window, cx` parameter order when both are present.
- Put callback parameters after `cx` in function signatures that accept callbacks.
- Inside `Entity<T>::read_with`, `update`, or `update_in` closures, use the inner `cx` passed to the closure, not an outer context captured from the caller.
- Avoid updating an entity while it is already being updated. Reentrant entity updates panic.
- Prefer `WeakEntity<T>` for long-running async work or mutually-referential entity graphs so dropped entities do not stay alive accidentally.
- Use `cx.listener(...)` for element handlers that need to mutate the current entity.
- Use `cx.emit(...)` and `cx.subscribe(...)` for entity events, and store returned `Subscription`s on the subscribing entity.
- Call `cx.notify()` after state changes that affect rendering.
- `cx.spawn(...)` and `cx.background_spawn(...)` return a `Task`. Dropping the handle cancels the work, so await it, detach it, or store it according to the intended lifetime.
- Use `Task::ready(value)` when a task only needs to provide an already-available value.

## GPUI Tests

In GPUI tests that rely on `run_until_parked()`, use GPUI executor timers instead of `smol::Timer::after(...)`.

```rust
cx.background_executor().timer(duration).await;
```

This keeps timeout and delay work scheduled on GPUI's dispatcher, so the test harness can drive it.

## GPUI Rendering

For redraw, invalidation, `deferred(...)`, and `AnyView::cached(...)` behavior, see [GPUI_RENDERING_MODEL.md](GPUI_RENDERING_MODEL.md).

## GPUI Scroll Containers

`overflow_scroll()` and `overflow_y_scroll()` require the `Div` to have a stable `.id(...)`.

```rust
div()
    .id("my-scroll-area")
    .overflow_y_scroll()
```

Do not apply GPUI scroll overflow helpers to anonymous `Div`s.

When the scroll area lives inside nested flex layouts, the parent chain must also provide a constrained height.

- Use `size_full()` on the viewport wrapper that is expected to fill the window body.
- Add `min_h_0()` to each intermediate flex child between the constrained viewport and the GPUI scroll node.
- Without this, GPUI tends to measure the scroll node at content height and `overflow_y_scroll()` will not produce a usable scroll range.

## GPUI Flex Layout — Width Resolution in Column Containers

Do not use `w_full()` (`width: 100%`) on flex items inside a `flex_col()` container to make them fill the container's width. Taffy only resolves percentage widths against a definite size. When a flex container's width comes from cross-axis stretch (the default), it is not considered definite for percentage resolution, so `width: 100%` on children resolves against a higher ancestor and produces wrong widths.

**Use instead**: omit the explicit width and let the default `align-self: stretch` fill the cross axis.

```rust
// Wrong — w_full() resolves against the wrong ancestor
div().flex_col()
    .child(div().w_full().flex().flex_row()...)

// Correct — stretch is the default and uses the definite container width
div().flex_col()
    .child(div().min_w_0().flex().flex_row()...)
```

Keep `min_w_0()` on flex items that contain truncated text or overflow content to prevent them from overflowing their container.

Text wrapping requires a definite width constraint. Use `.w(px(width))`, not `.max_w(px(width))`, on text containers — with only a max width, text wraps at viewport width.

For the column container itself to have a definite width (so its children can use stretch reliably), give it an explicit `w_full()` or absolute pixel width. Do not rely solely on cross-axis stretch being inherited transitively through multiple flex levels.

Do not combine `justify_between()` with `flex_1()` on a sibling to push a right-hand element to the far edge. With `flex_1()` consuming all free space, `justify-content: space-between` has no remaining space to distribute and behaves identically to `flex-start`. Use `flex_1()` on the left child alone — it naturally pushes the right child to the far edge without `justify_between`.
