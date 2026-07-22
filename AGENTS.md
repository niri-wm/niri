# Focus Ring Alpha Fade Animation

## Status

Feature implemented, tested, and working. Branch: `focus-ring-anim` (forked from v26.04).

## Summary

Animate the focus ring's alpha when window focus changes. When a window gains focus, the ring fades in (alpha 0→1). When it loses focus, it fades out (alpha 1→0). Previously the focus ring appeared/disappeared instantly.

---

## Files Changed

### 1. `niri-config/src/animations.rs` — Animation config type

**New struct `FocusRingAnim`:**

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FocusRingAnim(pub Animation);
```

A newtype wrapper around `Animation` (the config type), following the same pattern as `HorizontalViewMovementAnim`, `ConfigNotificationOpenCloseAnim`, `ScreenshotUiOpenAnim`, etc.

**Default:** 300ms `EaseOutQuad` — snappy and perceptible.

```rust
impl Default for FocusRingAnim {
    fn default() -> Self {
        Self(Animation {
            off: false,
            kind: Kind::Easing(EasingParams {
                duration_ms: 300,
                curve: Curve::EaseOutQuad,
            }),
        })
    }
}
```

**KDL parser:** Implements `knuffel::Decode` following the standard newtype pattern. This is what makes the animation configurable via `config.kdl` without recompiling.

**Modified `Animations`:** Added `pub focus_ring: FocusRingAnim` field, its `Default`, its `AnimationsPart` counterpart (`Option<FocusRingAnim>` with `#[knuffel(child)]`), and wired it into the `merge_clone!` macro for config file includes/overrides.

### 2. `src/layout/tile.rs` — Animation engine integration

**New fields on `Tile`:**

```rust
focus_ring_alpha_anim: Option<Animation>,
prev_focus_ring_is_active: bool,
```

- `focus_ring_alpha_anim` stores the running runtime `Animation` (the stateful engine type). `None` when idle.
- `prev_focus_ring_is_active` tracks the `is_active` value from the last frame, enabling transition detection.

**Modified `Tile::update_render_elements()`:**

Before the focus ring update, detects `is_active` transitions and manages the animation:

```rust
if is_active != self.prev_focus_ring_is_active {
    let target = if is_active { 1. } else { 0. };
    let current = self.focus_ring_alpha_anim
        .as_ref()
        .map(|a| a.clamped_value())
        .unwrap_or(if is_active { 0. } else { 1. });
    self.focus_ring_alpha_anim = Some(Animation::new(
        self.clock.clone(), current, target, 0.,
        self.options.animations.focus_ring.0,
    ));
}
self.prev_focus_ring_is_active = is_active;

let ring_alpha = self.focus_ring_alpha_anim
    .as_ref()
    .map(|a| a.clamped_value() as f32)
    .unwrap_or(1.);
```

The computed `ring_alpha` is multiplied with the existing expanded-progress alpha when passed to `FocusRing::update_render_elements()`:

```rust
ring_alpha * (1. - expanded_progress as f32)
```

**Modified `Tile::advance_animations()`:** Clears `focus_ring_alpha_anim` when the animation completes (via `anim.is_done()`).

### 3. `resources/default-config.kdl`

Added a commented-out example:

```kdl
// focus-ring {
//     duration-ms 150
//     curve "ease-out-expo"
// }
```

### 4. `niri-config/src/lib.rs`

Updated the inline `insta` test snapshot to include the `focus_ring` field.

---

## Architecture

```
focus change
    │
    ▼
Layout::refresh(is_active) ──► queue_redraw_all()
                                    │
                               (next frame)
                                    ▼
                              redraw()
                                    │
                                    ▼
                     update_render_elements(Some(output))
                                    │
                                    ▼
                     layout.update_render_elements(output)
                                    │
                                    ▼ (per tile)
                     Tile::update_render_elements(is_active)
                                    │
                     ┌─ is_active != prev?
                     │   → Animation::new(clock, current, target, 0, config)
                     │
                     ├─ ring_alpha = anim.clamped_value() ?? 1.0
                     │
                     └─ focus_ring.update(alpha: ring_alpha * expanded_alpha)
                                       │
                                       ▼
                          BorderRenderElement::update(alpha)
                                       │
                                       ▼
                          ShaderRenderElement (alpha uniform)
```

The animation is created inside `update_render_elements()` (called once per frame during rendering). It's then advanced and cleaned up in `advance_animations()`. Both methods are called from the rendering pipeline.

---

## Configuration

Users can configure the animation in `~/.config/niri/config.kdl`:

```kdl
animations {
    focus-ring {
        duration-ms 300
        curve "ease-out-quad"
    }
}
```

**Available curves:** `linear`, `ease-out-quad`, `ease-out-cubic`, `ease-out-expo`, `cubic-bezier` (with 4 control point values, e.g., `0.05 0.7 0.1 1`).

**Spring animation:**

```kdl
animations {
    focus-ring {
        spring damping-ratio=0.6 stiffness=500 epsilon=0.01
    }
}
```

**Disable (instant behavior):**

```kdl
animations {
    focus-ring {
        off
    }
}
```

Changes take effect after restarting niri.

---

## Edge Cases

- **Animation `off` flag:** Creates a zero-duration animation that jumps to the target alpha immediately, then clears itself on the next frame. Preserves the original instant behavior.
- **Fullscreen/maximized:** The focus ring is hidden when `expanded_progress == 1.0` (the maximize/fullscreen animation check in `Tile::render_inner()`). The alpha animation still runs but is invisible until the window un-fullscreens.
- **Interrupted transitions:** If focus changes mid-animation (e.g., rapidly Alt+Tabbing), the new animation starts from the current alpha value, preventing jumps.
- **Config merging:** Config file includes and partial overrides work correctly via the `merge_clone!` mechanism.

## Observed Issues

### Intermittent two-step / abrupt transition

The focus ring sometimes fades smoothly and sometimes appears to do a 2-step snap. This is inconsistent — it doesn't reproduce reliably. Possible causes to investigate:

- **`update_render_elements()` not being called every frame.** If a frame is skipped, the animation jumps ahead in one step, then resumes smoothly on the next frame. This could happen if the compositor skips rendering (e.g., no damage, VRR idle).
- **`clamped_value()` vs raw `value()`.** `clamped_value()` clamps to `to` after `clamped_duration`, while `value()` can extend beyond. If the clamping interacts poorly with frame timing, it could cause visible jumps.
- **Clock advancement granularity.** The clock advances in frame-sized chunks. If two frames are coalesced, the animation advances by 2× the per-frame delta, which can feel like a jump.
- **Startup flash.** On first frame, the active window fades from 0 because `prev_focus_ring_is_active` starts as `false` and the first `is_active = true` triggers a fade-in. This is a one-frame artifact at session start.

### Future investigation needed

- Add a `focus_ring_initialized` flag to skip animation on the very first frame of a tile's lifetime.
- Verify that `update_render_elements()` is called on every composed frame for every visible tile.
- Consider moving animation creation to `advance_animations()` or `refresh()` instead of `update_render_elements()` to decouple from rendering.
