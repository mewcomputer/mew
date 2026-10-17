# mew GPUI animations

Verified against the [pinned animation and spring sources](README.md#source-policy).

## Element identity and duration animations

`AnimationExt::with_animation(id, animation, animator)` wraps an element.
GPUI retains animation state under its element ID path. Stable IDs preserve
playback; a new ID restarts a duration animation. Keep each ID unique among live
siblings. The animator returns visual state; it should not issue protocol
commands, save preferences or mutate unrelated entities.

`Animation::new(duration)` runs once with linear easing. The pin provides
`with_easing`, `repeat`, `repeat_synced` and `with_max_fps`.
`with_animations` sequences duration animations and provides the active phase
index. Easing functions can overshoot; do not assume the resulting value always
lies in 0..1.

For example, an element-building fragment with GPUI imports is:

```rust
div().with_animation(
    "panel-enter",
    Animation::new(Duration::from_millis(220)).with_easing(gpui::ease_out_quint()),
    |element, delta| element.opacity(delta),
)
```

Zed's `ui` helpers such as `with_rotate_animation` are not GPUI APIs available to
mew. Use our existing element wrappers and helpers.

## Existing mew behavior

Sidebar and workbench transitions live in `shell/sidebar.rs`,
`shell/workbench.rs` and the transition helpers in `shell/helpers.rs`.
They retain the surface while it closes and use a changing animation ID.
Preserve endpoint geometry, width limits and interruption behavior when changing
them. Do not unmount the closing surface before its exit completes.

The transcript limits animated activity to the last visible row. Keep history
and completed rows quiet. Composer blink uses a weak shell and an epoch so
an obsolete timer cannot restart after focus changes.

Prefer `repeat_synced()` for repeating indicators that should share a phase.
Use an FPS cap where a slowly changing indicator needs fewer frames. Only mount
repeating animations while their state is active.

## Springs at this pin

`with_spring` and `SpringAnimation` are available. A stable spring element ID
preserves position and velocity while its target changes. A newly mounted spring
starts at its target unless an initial `from` value is supplied.

Choose springs when continuous retargeting serves the interaction. Existing
duration transitions do not need conversion solely because the API exists.
Keep dragging direct and use a settle animation only after release.

## Reduced motion and layout

The pinned animation wrappers respect `App::reduce_motion`: one-shot animations
render their end state, repeating animations their start state, and animation
frames stop being scheduled. Avoid manual animation loops that bypass this
behavior. Check the static state remains usable with reduced motion.

Constrain animated scroll containers and retain their IDs and list state.
Animating a parent can revisit uncached content every frame; read
[Rendering](GPUI_RENDERING_MODEL.md) before promising isolated repaint work.

## Verification

Test transition endpoints, width constraints, reverse/interrupt sequences and
whether closed/idle content stops requesting frames. Use the existing
`MEW_DESKTOP_FRAME_TRACE` probe to measure shell and transcript renders during
interactions. Choose timing in the context of mew's existing controls rather than
importing another application's timing rules.
