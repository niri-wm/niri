# Layer-Shell Animations

## What This Is

Implementation of smooth, configurable animations for Wayland layer-shell surfaces in Niri (waybar, dunst, rofi, etc.) achieving full parity with window open/close animations.

## Core Value

Layer-shell surfaces animate smoothly on open/close with configurable fade, slide, and scale effects, plus custom shader support.

## Requirements

### Validated

(None yet — ship to validate)

### Active

- [ ] Implement animation state in MappedLayer struct
- [ ] Add animation methods to MappedLayer (start_open_animation, start_close_animation, animated_alpha, animated_offset, animated_scale, are_animations_ongoing, advance_animations)
- [ ] Trigger open animation on layer map in layer_shell_handle_commit
- [ ] Trigger close animation on layer unmap in layer_destroyed
- [ ] Integrate layer animations with Niri::advance_animations
- [ ] Update MappedLayer::are_animations_ongoing to check new animations
- [ ] Rendering integration - apply animated alpha in render_normal
- [ ] Implement animated_offset for slide effect based on layer anchor
- [ ] Implement animated_scale for popin effect
- [ ] Add custom shader support for layer animations
- [ ] Implement snapshot buffers for animation rendering
- [ ] Handle animation interruption gracefully
- [ ] Test with real layer-shell apps (waybar, dunst, rofi)

### Out of Scope

- Per-surface animation rules via layer-rules — deferred to future
- Layer resize animations — layer surfaces don't resize dynamically

## Context

- Branch: feat/layer-anims
- Config already implemented in niri-config/src/animations.rs
- MappedLayer in src/layer/mapped.rs needs animation state
- Handler in src/handlers/layer_shell.rs needs animation triggers
- Window animations in src/layout/opening_window.rs and closing_window.rs are reference implementations

## Constraints

- **Tech Stack**: Rust, Smithay, niri animation system
- **Compatibility**: Must not break existing layer-shell functionality
- **Pattern**: Follow existing window animation patterns for consistency
- **Testing**: Manual testing with waybar, dunst, rofi

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Use unified Animation type | Consistent with existing niri patterns | — Pending |
| Fade + slide + scale via rendering interpolation | Niri approach vs Hyprland's style enum | — Pending |
| Global config only | Per-surface rules deferred to simplify initial implementation | — Pending |

---

*Last updated: 2026-02-16 after initialization*
