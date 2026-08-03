# GPUI Development Reference

Curated docs pulled from [zedra](https://github.com/tanlethanh/zedra/tree/main/docs) — a GPUI-based app.
Selected for relevance to **GPUI development** in general, not just the zedra project.

All docs carry a source-note prefix linking back to the original zedra repository.

## Core GPUI Concepts

| Doc | Why it matters |
|-----|----------------|
| [GPUI_RENDERING_MODEL.md](GPUI_RENDERING_MODEL.md) | The mental model for when/why views rerender: `cx.notify()`, the frame pipeline (invalidate → traverse → layout → prepaint → paint → present), dirty propagation, and subtree caching with `deferred(...)` / `AnyView::cached(...)`. Essential for reasoning about performance. |
| [GPUI_ANIMATIONS.md](GPUI_ANIMATIONS.md) | GPUI's animation system: the `Animation` element wrapper, `ElementId` lifecycle, four core patterns (enter, re-triggered/exit, looping, sequenced), easing functions, gesture-driven motion, and timing defaults. |
| [GPUI_FOCUS_INPUT_KEYBOARD.md](GPUI_FOCUS_INPUT_KEYBOARD.md) | Focus management (`.track_focus()` vs `.manual_focus()`), platform text input via `Window::handle_input(...)`, and keyboard coordination. Key for any app with text input. |

## App Structure & Patterns

| Doc | Why it matters |
|-----|----------------|
| [CONVENTIONS.md](CONVENTIONS.md) | GPUI coding conventions: entity/task/`cx` usage, flex layout pitfalls (width resolution in columns, `w_full()` misuse), scroll container requirements (stable `.id()`, height constraints), and testing patterns. High-density best-practices reference. |
| [THEMING.md](THEMING.md) | Theme system: `ThemePalette` / `ThemeBundle` layers, `theme::palette(cx)` accessor pattern, theme change events, and a checklist for new screens. |
| [DESIGN.md](DESIGN.md) | Visual design language: color system with semantic accents, monospace-first typography, spacing rhythm (8/12/16), and component patterns for panels, cards, inputs, buttons. Includes git view guidance. |

## Advanced & Platform Integration

| Doc | Why it matters |
|-----|----------------|
| [GPUI_CUSTOM_EFFECT.md](GPUI_CUSTOM_EFFECT.md) | How to extend GPUI with custom GPU/Metal render effects after scene render but before present. Includes the `MetalRenderEffect` trait, blending models, and a water-droplet example. Rare GPUI graphics programming reference. |
| [GPUI_NATIVE_PRESENTATIONS.md](GPUI_NATIVE_PRESENTATIONS.md) | Pattern for mixing native platform UI (alerts, sheets, notifications) with GPUI-rendered views. Shows shared state between main window and detached sheet windows, scroll gesture handoff, and embedded GPUI surfaces. |

## Tooling

| Doc | Why it matters |
|-----|----------------|
| [DEVTOOL.md](DEVTOOL.md) | HTTP-based devtool server for driving GPUI UI programmatically — `GET /elements` for hitbox inspection, `POST /tap` for synthetic input. Useful for testing and automation. Portable pattern for any GPUI app. |

## What was excluded

The following zedra docs were reviewed but skipped as they're project-specific, mobile-only, or not GPUI-related:

- **GPUI_ANDROID*.md** (5 files) — Android/Vulkan backend specifics
- **GPUI_MOBILE_*.md** (2 files) — Mobile touch/gesture RFCs (valuable if targeting mobile)
- **IOS_WORKFLOW.md** — iOS build pipeline and Xcode FFI
- **GPUI_INPUT_DICTATION.md** — iOS dictation integration
- **ARCHITECTURE.md** — zedra's iroh/RPC architecture (removed)
- **GET_STARTED.md** — Zedra-specific dev setup
- **MANUAL_TEST.md** — Project test plan
- **PROTOCOL_SPECS.md**, **NETWORK_TRANSPORT.md**, **RELAY.md** — Zedra RPC/infra
- **TELEMETRY.md**, **RELEASE.md**, **LIVE_ACTIVITY.md** — Ops/platform features
- **MANAGED_AGENTS.md**, **EXTENSIONS_SYSTEM.md**, **AI_AGENTS_CLI_INTEGRATION.md** — Zedra agent system
- **DELTA_INTEGRATION.md**, **NATIVE_TEXT_SELECTION_TEST_PLAN.md**, **WRITING.md** — Project internals

If you decide to target mobile later, the GPUI_MOBILE_*.md and GPUI_ANDROID*.md docs are worth revisiting.
