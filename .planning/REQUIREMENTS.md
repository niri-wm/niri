# Requirements: Layer-Shell Animations

**Defined:** 2026-02-16
**Core Value:** Layer-shell surfaces animate smoothly on open/close with configurable fade, slide, and scale effects.

## v1 Requirements

### Animation Infrastructure

- [x] **ANIM-01**: Export `layer_animation` module in `src/layer/mod.rs`
- [x] **ANIM-02**: Add `open_animation: Option<LayerAnimation>` field to `MappedLayer`
- [x] **ANIM-03**: Add `close_animation: Option<LayerAnimation>` field to `MappedLayer`
- [x] **ANIM-04**: Add `start_open_animation()` method to `MappedLayer`
- [x] **ANIM-05**: Add `start_close_animation()` method to `MappedLayer`
- [x] **ANIM-06**: Add `are_animations_ongoing()` method checking both animations
- [x] **ANIM-07**: Add `advance_animations()` method to clear completed animations

### Animation Triggers

- [x] **TRIG-01**: Trigger open animation in `layer_shell_handle_commit()` when layer maps
- [x] **TRIG-02**: Trigger close animation in `layer_destroyed()` when layer unmaps
- [x] **TRIG-03**: Pass animation config (`config.animations.layer_open/close`) to animation constructors
- [x] **TRIG-04**: Handle animation interruption (open while closing, etc.)

### Animation Integration

- [x] **INT-01**: Add layer animations to `Niri::advance_animations()`
- [x] **INT-02**: Update `MappedLayer::are_animations_ongoing()` to check new animations
- [x] **INT-03**: Ensure frame callbacks sent during layer animations

### Rendering Integration

- [x] **REND-01**: Render with animation in `MappedLayer::render_normal()` when animating
- [x] **REND-02**: Apply animated alpha to layer rendering
- [x] **REND-03**: Use `LayerAnimation::render()` for animated frames
- [x] **REND-04**: Fallback to normal rendering when animation completes

### Custom Shaders

- [x] **SHAD-01**: Add custom shader support functions for layer animations
- [x] **SHAD-02**: Load custom shaders from config on startup
- [x] **SHAD-03**: Reload shaders on config change

### Edge Cases

- [ ] **EDGE-01**: Handle rapid open/close cycles gracefully
- [ ] **EDGE-02**: Handle animation when layer is already visible
- [ ] **EDGE-03**: Handle multiple monitors (per-output animation state)

## v2 Requirements

(None yet - defer as needed)

## Out of Scope

| Feature | Reason |
|---------|--------|
| Per-surface animation rules via layer-rules | Deferred to future phase for complexity |
| Layer resize animations | Layer surfaces don't resize dynamically |

## Traceability

| Requirement | Phase | Status |
|-------------|-------|--------|
| ANIM-01 - ANIM-07 | Phase 1 | Complete |
| TRIG-01 - TRIG-04 | Phase 2 | Complete |
| INT-01 - INT-03 | Phase 2 | Complete |
| REND-01 - REND-04 | Phase 3 | Complete |
| SHAD-01 - SHAD-03 | Phase 4 | Complete |
| EDGE-01 - EDGE-03 | Phase 5 | Pending |

**Coverage:**
- v1 requirements: 21 total
- Mapped to phases: 21
- Complete: 17
- Pending: 4
- Unmapped: 0 ✓

---
*Requirements defined: 2026-02-16*
*Last updated: 2026-02-17 after phase 4 completion*
