# Roadmap: Layer-Shell Animations

**Created:** 2026-02-16
**Phases:** 5
**Requirements:** 21

## Summary

Implementation of smooth layer-shell surface animations achieving full parity with window animations. The LayerAnimation struct already exists - the work is integration and wiring.

## Phase Overview

| # | Phase | Goal | Requirements | Success Criteria |
|---|-------|------|--------------|------------------|
| 1 | Animation Infrastructure | Add animation state and methods to MappedLayer | ANIM-01 to ANIM-07 | MappedLayer can store and manage open/close animations |

**Plans:**
- [x] 01-01-PLAN.md — Export module, add fields and methods to MappedLayer
- [x] 02-01-PLAN.md — Trigger open/close animations on map/destroy, integrate with advance_animations
- [x] 03-01-PLAN.md — Integrate layer animations into rendering pipeline
- [x] 04-01-PLAN.md — Wire layer shaders at startup and hot-reload
| 2 | Animation Triggers | Wire up triggers in layer shell handler | TRIG-01 to TRIG-04, INT-01 to INT-03 | Animations start on map/unmap events |
| 3 | Rendering Integration | Render layers with animation effects | REND-01 to REND-04 | Layers animate visually on open/close |
| 4 | Custom Shaders | Add shader support for layer animations | SHAD-01 to SHAD-03 | Custom GLSL shaders work for layers |
| 5 | Edge Cases & Testing | Handle edge cases and manual testing | EDGE-01 to EDGE-03 | Works reliably with real apps |

**Plans:**
- [x] 05-01-PLAN.md — Fix position bug (remove bob_offset)
- [x] 05-02-PLAN.md — Fix open animation trigger
- [x] 05-03-PLAN.md — Fix close animation with snapshot

## Phase Details

### Phase 1: Animation Infrastructure

**Goal:** Add animation state and methods to MappedLayer

**Requirements:**
- ANIM-01: Export `layer_animation` module in `src/layer/mod.rs`
- ANIM-02: Add `open_animation: Option<LayerAnimation>` field to `MappedLayer`
- ANIM-03: Add `close_animation: Option<LayerAnimation>` field to `MappedLayer`
- ANIM-04: Add `start_open_animation()` method to `MappedLayer`
- ANIM-05: Add `start_close_animation()` method to `MappedLayer`
- ANIM-06: Add `are_animations_ongoing()` method checking both animations
- ANIM-07: Add `advance_animations()` method to clear completed animations

**Success Criteria:**
1. `MappedLayer` struct has animation state fields
2. Animation can be started via method calls
3. Animation progress can be queried
4. Completed animations can be cleared

**Implementation Notes:**
- Follow pattern from `OpeningWindow`/`ClosingWindow` in layout module
- Use existing `LayerAnimation` struct from `src/layer/layer_animation.rs`
- `MappedLayer` already has `clock: Clock` field

---

### Phase 2: Animation Triggers

**Goal:** Wire up triggers in layer shell handler

**Requirements:**
- TRIG-01: Trigger open animation in `layer_shell_handle_commit()` when layer maps
- TRIG-02: Trigger close animation in `layer_destroyed()` when layer unmaps
- TRIG-03: Pass animation config (`config.animations.layer_open/close`) to animation constructors
- TRIG-04: Handle animation interruption (open while closing, etc.)
- INT-01: Add layer animations to `Niri::advance_animations()`
- INT-02: Update `MappedLayer::are_animations_ongoing()` to check new animations
- INT-03: Ensure frame callbacks sent during layer animations

**Success Criteria:**
1. Layer surfaces animate when appearing (waybar on startup)
2. Layer surfaces animate when disappearing (rofi dismiss)
3. Animation config is respected (duration, curve)
4. Animations advance each frame

**Implementation Notes:**
- Trigger open animation when `was_unmapped` is true in `layer_shell_handle_commit()`
- Trigger close animation in `layer_destroyed()` before removing from map
- Handle case where animation already exists (interruption)

---

### Phase 3: Rendering Integration

**Goal:** Render layers with animation effects

**Requirements:**
- REND-01: Render with animation in `MappedLayer::render_normal()` when animating
- REND-02: Apply animated alpha to layer rendering
- REND-03: Use `LayerAnimation::render()` for animated frames
- REND-04: Fallback to normal rendering when animation completes

**Success Criteria:**
1. Opening layers fade/scale in smoothly
2. Closing layers fade/scale out smoothly
3. Animation blends with normal rendering
4. No visual glitches during animation

**Implementation Notes:**
- Check `are_animations_ongoing()` in `render_normal()`
- Use `LayerAnimation::render()` which handles shader and fallback
- Default animation includes scale transform (0.5→1.0 for open)

---

### Phase 4: Custom Shaders

**Goal:** Add shader support for layer animations

**Requirements:**
- SHAD-01: Add custom shader support functions for layer animations
- SHAD-02: Load custom shaders from config on startup
- SHAD-03: Reload shaders on config change

**Success Criteria:**
1. Custom GLSL shaders work for layer-open animation
2. Custom GLSL shaders work for layer-close animation
3. Shaders reload on config hot-reload

**Implementation Notes:**
- Follow pattern from window animation shaders in `src/render_helpers/shaders/`
- Use `ProgramType::Open` and `ProgramType::Close` for layer shaders

**Plans:**
- [x] 04-01-PLAN.md — Wire layer shaders at startup and hot-reload

---

### Phase 5: Edge Cases & Testing

**Goal:** Handle edge cases and manual testing

**Requirements:**
- EDGE-01: Handle rapid open/close cycles gracefully
- EDGE-02: Handle animation when layer is already visible
- EDGE-03: Handle multiple monitors (per-output animation state)

**Success Criteria:**
1. Rapid toggling doesn't cause crashes
2. Re-showing animating layer works correctly
3. Animations work on all outputs

**Testing:**
- Test with waybar (panel)
- Test with dunst (notifications)
- Test with rofi/wofi (launchers)

---

## Notes

- **Parallelization:** Phases 1-3 are sequential (each builds on previous)
- **Phase 4** can start after Phase 3 is complete
- **Phase 5** is testing/cleanup - can run in parallel with verification
- Config already exists: `layer_open` and `layer_close` in `niri-config/src/animations.rs`

---

*Roadmap created: 2026-02-16*
*Last updated: 2026-02-17 after phase 3 completion*
