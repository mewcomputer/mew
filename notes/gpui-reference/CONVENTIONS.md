# mew GPUI conventions

Verified against the [pinned GPUI source](README.md#source-policy). Repository
Rust conventions remain in [AGENTS.md](../../AGENTS.md).

## Ownership and module boundaries

The desktop path is GPUI -> transport -> daemon -> session -> agent -> provider.
`DesktopShell` owns presentation, drafts, focus and retained list state.
`ShellModel` projects the shared client state. Protocol interpretation and
reconnect receipts belong in `mew-client-core`; session and agent execution
belong in the daemon.

Relevant mew files:

- `apps/mew-desktop/src/shell/lifecycle.rs`: client connection, shutdown and theme reload.
- `shell/types.rs`: retained view state, subscriptions, lists and profiler.
- `shell/session_requests.rs`, `drafts.rs`, `review.rs`: request and navigation identities.
- `shell/browser.rs` and `native/cef-host`: owned CEF pages and native embedding.
- `terminal/terminal_view.rs`: terminal projection and GPUI entity.

Keep CEF page callbacks scoped to connection profile, session and page instance.
A late response must not replace another draft, attachment, connection or page.
Use the existing generation and owner checks rather than adding another identity.

## Entities and notifications

Use `window, cx` in that order. Use the closure's inner context for entity
reads/updates; updating an entity reentrantly panics.

`cx.listener(...)`, `Entity::update` and deferred callbacks do not automatically
notify. After a visible state change, notify the entity that owns the state,
including error and early-return paths. Retain subscriptions for the lifetime
of the observation.

A view entity alone does not isolate render work. Read
[Rendering](GPUI_RENDERING_MODEL.md) before changing invalidation or caching.

## Tasks and blocking work

Keep network waits, daemon startup probes, filesystem scans and process waits
off the GPUI foreground thread. Existing client transport work runs on a
dedicated Tokio thread. Cancel both receive waits and pending command sends
before joining that thread.

GPUI tasks cancel when their handles are dropped. Await, store or deliberately
detach each task according to its lifetime. Use weak entities for background
completion and retained timers, and validate the captured connection/request
identity before applying results. A weak handle prevents retention; it does not
make a stale response belong to the current session.

Use executor timers in GPUI tests that depend on scheduler pumping. Retain the
existing shutdown path for app-owned daemons and CEF pages.

## Layout, scrolling and lists

`overflow_y_scroll()` requires a stateful element with a stable `.id(...)`.
Constrain the viewport and use `min_h_0()` through flexible vertical children;
use `min_w_0()` where text must shrink or truncate.

Percentage sizes need an appropriate containing size. Diagnose the actual
layout chain before adding `w_full()` or a fixed width; there is no blanket ban
on percentage widths inside columns. Check wrapping at both narrow and wide
window sizes.

Retain `ListState` and `ScrollHandle` on the owning entity. Use `list` for
variable-height transcript/review rows and `uniform_list` only when row heights
are actually uniform. Keep IDs tied to the domain item rather than its changing
visible index. Preserve follow-tail and history-prepend anchors when updating
transcript rows.

## Tests and source verification

Test the user-visible failure before fixing it. Use real GPUI entity updates
for notification/focus/lifetime behavior and real terminal snapshots for terminal
projection. Pure helper tests remain useful for ranges and state transitions,
but a notification classifier alone cannot establish how many views render.

Read [Verification and tooling](DEVTOOL.md) for the narrow desktop checks and
native smoke procedure. Check external examples against the dependency source
before using an API or feature name.
