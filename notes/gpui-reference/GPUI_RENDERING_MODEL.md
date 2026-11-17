# GPUI Rendering Model

Verified against the [pinned GPUI source](README.md#source-policy), especially
`view.rs`, `window.rs` and `app/context.rs`. This guide describes mew's desktop.

A practical reference for GPUI's rendering model, based on the GPUI implementation in Zed's `crates/gpui/src` and how Zed itself uses `Window`, `AnyView`, `deferred(...)`, and `cached(...)`.

## Mental Model

- A GPUI `Window` is the main redraw boundary.
- Most app UI for a workspace lives inside one native window.
- `cx.notify()` usually schedules a new frame for the affected window.
- A new frame traverses the root view tree again.
- Uncached views encountered during that traversal will have `render()` called again.

Short version:

```text
entity notify -> window redraw -> root tree traversal -> uncached child views rerender
```

## Frame Pipeline

For each window draw, GPUI performs these phases:

1. Invalidate dirty entities for the window.
2. Traverse the root view tree.
3. Request layout.
4. Prepaint.
5. Paint.
6. Present the scene.

The important consequence is that GPUI does not incrementally rerender arbitrary subtrees by default. The window redraws from the root, and subtree reuse only happens when GPUI has an explicit reuse boundary.

## What `cx.notify()` Does

`cx.notify()` does not directly mean "rerender the whole app".

What it actually does:

- Marks one entity as changed.
- Notifies windows that previously tracked that entity during rendering.
- Marks the affected view and its ancestor views dirty in that window.
- Schedules another frame for that window.

Practical effect in a single-window app:

- It often feels like "rerender the whole app" because the window redraws from the root.
- The real boundary is the window, not the entire process.

## Dependency Tracking

During rendering, GPUI records which entities a view accessed.

Later, if one of those entities calls `cx.notify()`, GPUI knows which windows and views need another frame.

This matters because a cached view is only reusable if:

- the view itself is not dirty, and
- none of the entities it accessed during its last render caused it to become dirty.

## Dirty Propagation

When a view is invalidated, GPUI marks that view and its ancestor views dirty inside the window dispatch tree.

That means:

- a leaf update can make parent views dirty,
- the next frame starts at the root again,
- and parent render paths can still cause sibling subtrees to be traversed.

This is why separating state into another entity is helpful but not sufficient to stop rerenders on its own.

## `deferred(...)`

`deferred(...)` is a paint-order tool, not a rerender boundary.

What it does:

- keeps the child in the current layout tree,
- delays drawing so it can appear above normal content,
- helps with overlays, menus, popovers, and similar layers.

What it does not do:

- it does not isolate the child into a separate window,
- it does not stop the child from participating in layout,
- it does not prevent parent redraws,
- it does not prevent `render()` from being called again.

So `deferred(...)` is useful for overlay layering, but it is not enough to stop heavy content from rerendering during animation.

## `AnyView::cached(...)`

`AnyView::cached(...)` is GPUI's built-in subtree reuse boundary.

What it means:

- GPUI may reuse the previous frame's prepaint and paint output for that view.
- If reuse succeeds, GPUI skips calling that view's `render()` for the frame.

Reuse is allowed only when all of these still match:

- bounds
- content mask
- text style
- the view is not dirty
- the window is not in `refresh()` mode

Important:

- cache does not stop the window from redrawing,
- cache only lets one subtree reuse its previous output during that redraw.

## `Window::refresh()`

`window.refresh()` is broader than `cx.notify()`.

It marks the window for another frame and disables cached-view reuse for that frame.

Use it when the whole window needs redraw behavior that should bypass normal subtree reuse.

## Mew's current boundaries

`DesktopShell` is the root view. Its terminal is a child entity, and its browser
pump retains native CEF work. Mew currently uses ordinary child views without
GPUI `.cached(...)` boundaries. A terminal notification can therefore revisit
the shell and transcript as well as the terminal.

The streaming event classifier avoids unnecessary explicit shell notifications;
that alone does not prove that the shell never renders. Measure root and
transcript counts with the frame probe before changing entity boundaries.

Both `Entity<T>::cached(style)` and `AnyView::cached(style)` exist at this pin.
If a measured hot subtree needs reuse, provide its real layout contract and
test dependencies, theme changes, resize, focus and input registration. Do not
introduce caching merely to make an entity look isolated.

The terminal also has an application-owned row cache. VT cells, local selection
and theme defaults all feed that projection. A visible local mutation must
invalidate the row projection as well as notify GPUI; a redraw cannot fix stale
cached row colors or cell-to-text mappings.

## Practical Rules

- Mew displays sessions from multiple workspaces in one GPUI window.
- Keep drawers, overlays, modals, and panels inside that window.
- Do not expect `deferred(...)` or a separate entity alone to stop rerenders.
- If a heavy subtree must avoid rerender during unrelated animation, it needs a reuse boundary.
- The explicit view reuse boundaries are `Entity<T>::cached(...)` and `AnyView::cached(...)`.

If you choose not to use cache, that is valid, but the expected behavior changes:

- content may rerender during overlay animation frames,
- tests should not assert "content never rerenders while animating",
- performance work should focus on making rerender cheap rather than assuming it can be avoided.

## Rule of Thumb

Use this as the default model when reasoning about UI updates:

```text
cx.notify() -> redraw affected window next frame
redraw -> traverse root tree again
uncached views -> render again
cached views -> may reuse prior subtree output
```
