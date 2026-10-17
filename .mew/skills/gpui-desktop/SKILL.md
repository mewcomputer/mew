---
name: gpui-desktop
description: Build, debug, or audit mew's native GPUI desktop using its pinned framework APIs, existing client boundaries, and behavior tests. Use for apps/mew-desktop views, input, rendering, themes, terminal projection, and CEF integration.
---

# mew GPUI desktop

Use this skill for the native Rust desktop. The browser React client and Swift
mobile app have different UI contracts.

## Start from the pin and owning layer

Read the GPUI entries in `apps/mew-desktop/Cargo.toml`, then the
[reference index and source policy](../../../notes/gpui-reference/README.md).
Verify non-obvious APIs against that exact revision. GPUI-CE and Zedra examples
can use different APIs; Zed's `ui` crate is not a mew dependency.

Read the affected view and its behavior tests before changing it. Keep daemon
and shared-client work in their existing crates; GPUI owns presentation, focus,
drafts and native surfaces. Start with
[conventions and ownership](../../../notes/gpui-reference/CONVENTIONS.md).

## Load the relevant guide

- Notification, redraw cost or cached terminal rows:
  [rendering](../../../notes/gpui-reference/GPUI_RENDERING_MODEL.md).
- Text editing, keyboard actions, focus or native IME:
  [focus and input](../../../notes/gpui-reference/GPUI_FOCUS_INPUT_KEYBOARD.md).
- Animation identity, interrupted transitions, springs or reduced motion:
  [animations](../../../notes/gpui-reference/GPUI_ANIMATIONS.md).
- Theme tokens, appearance or child-view color updates:
  [theming](../../../notes/gpui-reference/THEMING.md).
- Controls, density or layout decisions:
  [design conventions](../../../notes/gpui-reference/DESIGN.md).
- Test harness, native smoke or frame measurements:
  [verification and tooling](../../../notes/gpui-reference/DEVTOOL.md).

Read only the guides that affect the change. They are the maintained reference;
do not duplicate their content into another manual.

## Apply and verify

Reproduce a reported gap in a behavior test before fixing it. State mutations
need explicit notification, including deferred failures. Application-owned
caches also need invalidation when their inputs change. A separate entity does
not by itself isolate root render work.

Keep blocking work off the foreground thread. Retain subscriptions and task
ownership intentionally, use weak completion handles, and reject obsolete
connection/session/page results through existing identities.

Test framework behavior with actual GPUI entities/shaping and transport behavior
with real local connections. Run the affected tests first, then the desktop
suite, strict Clippy and native build. Use isolated settings/session directories
for shell or daemon smoke tests. Follow the tooling guide for broader gates and
native checks; say what remains unverified.

For an audit, distinguish reproducible correctness gaps from measured performance
findings and optional structural suggestions. Fix the verified problem at its
owning layer and retain unrelated implementations. Record source/revision drift
in the reference index when changing the GPUI dependency.
