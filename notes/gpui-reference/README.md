# mew GPUI development reference

These guides describe mew's native desktop client against GPUI revision
`322cd509dcccaee1f6bc44725d270598ae8171ad`, verified on 2026-10-17.
[apps/mew-desktop/Cargo.toml](../../apps/mew-desktop/Cargo.toml) owns the dependency
pins, including the test-support dependency. Keep all GPUI entries on one revision.

Start with the [gpui-desktop skill](../../.mew/skills/gpui-desktop/SKILL.md).
Read the guides relevant to the change:

| Guide | Use it for |
| --- | --- |
| [Conventions](CONVENTIONS.md) | mew ownership boundaries, entities, tasks, subscriptions, layout and lists |
| [Rendering](GPUI_RENDERING_MODEL.md) | invalidation, subtree reuse and terminal row caches |
| [Focus and input](GPUI_FOCUS_INPUT_KEYBOARD.md) | keyboard dispatch, native text input, UTF-16 ranges and IME geometry |
| [Animations](GPUI_ANIMATIONS.md) | element identity, timing, springs and reduced motion |
| [Theming](THEMING.md) | manifest tokens, appearance and child-view updates |
| [Design](DESIGN.md) | existing desktop controls and layout conventions |
| [Verification and tooling](DEVTOOL.md) | GPUI tests, native smoke checks and frame profiling |

## Source policy

Use the exact dependency revision when checking APIs. A local Cargo git checkout
is useful, but its directory is a cache, not a stable project path. Restore it
with `cargo fetch --locked` when needed; `cargo metadata --locked --format-version 1`
reports each package's `manifest_path`. Use Cargo's Git CLI fetch option if this
machine's GitHub URL rewriting requires it.

Primary sources at the pin:

- [GPUI ownership and contexts](https://github.com/zed-industries/zed/blob/322cd509dcccaee1f6bc44725d270598ae8171ad/crates/gpui/src/_ownership_and_data_flow.rs)
  and [contexts guide](https://github.com/zed-industries/zed/blob/322cd509dcccaee1f6bc44725d270598ae8171ad/crates/gpui/docs/contexts.md).
- [View caching](https://github.com/zed-industries/zed/blob/322cd509dcccaee1f6bc44725d270598ae8171ad/crates/gpui/src/view.rs)
  and [window rendering and input](https://github.com/zed-industries/zed/blob/322cd509dcccaee1f6bc44725d270598ae8171ad/crates/gpui/src/window.rs).
- [Interactive elements](https://github.com/zed-industries/zed/blob/322cd509dcccaee1f6bc44725d270598ae8171ad/crates/gpui/src/elements/div.rs),
  [entity input handlers](https://github.com/zed-industries/zed/blob/322cd509dcccaee1f6bc44725d270598ae8171ad/crates/gpui/src/input.rs)
  and [key dispatch](https://github.com/zed-industries/zed/blob/322cd509dcccaee1f6bc44725d270598ae8171ad/crates/gpui/docs/key_dispatch.md).
- [Animation wrappers](https://github.com/zed-industries/zed/blob/322cd509dcccaee1f6bc44725d270598ae8171ad/crates/gpui/src/elements/animation.rs),
  [springs](https://github.com/zed-industries/zed/blob/322cd509dcccaee1f6bc44725d270598ae8171ad/crates/gpui/src/spring.rs)
  and [profiler](https://github.com/zed-industries/zed/blob/322cd509dcccaee1f6bc44725d270598ae8171ad/crates/gpui/src/profiler.rs).

[GPUI Community Edition](https://github.com/gpui-ce/gpui-ce) is a useful secondary
source of examples. It is a separate fork with diverging APIs; verify examples
against our pin. Zed's `ui` crate is an application component library, not a mew
dependency or a set of GPUI extension methods available here.

## Historical references

This directory originally contained imported
[Zedra](https://github.com/tanlethanh/zedra/tree/main/docs) guides. The active
guides above are now mew-specific. The following originals remain as historical
fork designs, not supported mew APIs:

- [Custom render effects](GPUI_CUSTOM_EFFECT.md): fork-specific GPU hooks.
- [Native presentations](GPUI_NATIVE_PRESENTATIONS.md): Zedra's native mobile
  sheets and platform bridge.

When changing the GPUI pin, recheck the linked source, examples, feature gates
and documented limitations, then run the verification guide. Preserve the source
and revision distinction when importing new community material.
