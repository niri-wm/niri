# Feature Landscape: Layer-Shell Animations in Niri

**Domain:** Wayland Compositor - Layer-Shell Surface Animations
**Researched:** 2026-02-16
**Confidence:** HIGH

## Executive Summary

Layer-shell animations (for waybar, dunst, rofi, etc.) have foundational infrastructure in place but are **not yet integrated** into the compositor lifecycle. The `LayerAnimation` struct exists with full render logic, and configuration types (`LayerOpenAnim`, `LayerCloseAnim`) are defined, but the glue code that triggers animations on surface map/unmap events is missing.

The implementation follows the same pattern as window animations (`OpeningWindow`, `ClosingWindow`), making the remaining work straightforward but requiring changes across multiple files.

## What's Already Implemented

### 1. Animation Struct and Rendering (`src/layer/layer_animation.rs`)

The `LayerAnimation` struct is fully implemented with:

- **Constructors**: `new_open()` and `new_close()` methods that take an `Animation`
- **State tracking**: `is_done()`, `clamped_value()` methods matching window animation pattern
- **Render logic**: Full implementation supporting:
  - Custom shaders via `ProgramType::Open` / `ProgramType::Close`
  - Fallback non-shader animations (alpha fade + scale)
  - Offscreen buffer rendering for both open and close states
- **181 lines** of battle-tested code following the exact pattern of `OpeningWindow` and `ClosingWindow`

```rust
// src/layer/layer_animation.rs lines 35-56
impl LayerAnimation {
    pub fn new_open(anim: Animation) -> Self { ... }
    pub fn new_close(anim: Animation) -> Self { ... }
    pub fn is_done(&self) -> bool { ... }
    pub fn clamped_value(&self) -> f64 { ... }
    pub fn render(...) -> anyhow::Result<(LayerAnimationRenderElement, OffscreenData)> { ... }
}
```

### 2. Animation Configuration (`niri-config/src/animations.rs`)

Configuration types are fully defined:

- **`LayerOpenAnim`**: Default 200ms, `EaseOutExpo` curve, optional `custom_shader`
- **`LayerCloseAnim`**: Default 150ms, `EaseOutQuad` curve, optional `custom_shader`

These mirror `WindowOpenAnim`/`WindowCloseAnim` structure:

```rust
// niri-config/src/animations.rs lines 207-246
pub struct LayerOpenAnim {
    pub anim: Animation,
    pub custom_shader: Option<String>,
}

pub struct LayerCloseAnim {
    pub anim: Animation,
    pub custom_shader: Option<String>,
}
```

### 3. Shader Infrastructure (`src/render_helpers/shaders/mod.rs`)

The shader system already supports layer animations:

- `ProgramType::Open` - Custom open shader (loads from config)
- `ProgramType::Close` - Custom close shader (loads from config)
- Both support user-provided GLSL fragment shaders

### 4. Animation Core (`src/animation/mod.rs`)

The core `Animation` struct provides:
- Spring physics, easing curves, and deceleration modes
- Time-based value calculation
- Config hot-reloading via `replace_config()`

## What's NOT Implemented (Needs Work)

### 1. Animation Triggering on Surface Map/Unmap

**Status:** NOT IMPLEMENTED

No code currently calls `LayerAnimation::new_open()` or `new_close()` when layer surfaces are mapped or unmapped.

**Required in:** `src/handlers/layer_shell.rs`

Window animation triggers in `src/handlers/compositor.rs`:
```rust
// compositor.rs line 235 - window open
self.niri.layout.start_open_animation_for_window(&window);

// compositor.rs line 276 - window close  
.start_close_animation_for_window(renderer, &window, blocker);
```

Layer-shell needs equivalent triggers in `layer_shell.rs` when:
- New layer surface maps (`new_layer_surface()` callback)
- Layer surface unmaps/destroys (`layer_destroyed()` callback)

### 2. Animation State in MappedLayer

**Status:** NOT IMPLEMENTED

`MappedLayer` in `src/layer/mapped.rs` does not store animation state. Compare to window handling where `Tile` stores `opening_window: Option<OpenAnimation>` and `closing_window: Option<ClosingWindow>`.

**Required changes:**
- Add `opening_animation: Option<LayerAnimation>` to `MappedLayer`
- Add `closing_animation: Option<LayerAnimation>` to `MappedLayer`
- These would mirror how `src/layout/tile.rs` stores `Option<OpenAnimation>` / `Option<ClosingWindow>`

### 3. Animation Advancement

**Status:** NOT IMPLEMENTED

`MappedLayer` has `are_animations_ongoing()` but only for `baba_is_float`, not for open/close animations.

**Required changes:**
- Add animation advancement in `MappedLayer::advance_animations()` 
- Call it from `src/niri.rs` `advance_animations()` alongside other systems
- Report animation completion status for render loop

Current stub in `src/layer/mapped.rs`:
```rust
// line 108-110
pub fn are_animations_ongoing(&self) -> bool {
    self.rules.baba_is_float
}
```

### 4. Render Integration

**Status:** NOT IMPLEMENTED

No code calls `LayerAnimation::render()` during output rendering. Compare to window rendering in `src/layout/tile.rs` which calls `self.opening.render()` or `self.closing.render()`.

**Required changes:**
- In render path (likely `MappedLayer::render_normal()`), check for active animation
- If animation exists, call `render()` instead of direct surface rendering
- Handle both the animation element and the offscreen data properly

### 5. Animation Completion Handling

**Status:** NOT IMPLEMENTED

No cleanup logic when animation completes. Windows handle this by:
- Moving from opening state → normal render
- Keeping closing state until animation done, then actually removing

## Integration Gaps Summary

| Gap | Location | Pattern to Follow |
|-----|----------|-------------------|
| Trigger open animation | `handlers/layer_shell.rs::new_layer_surface()` | `compositor.rs::start_open_animation_for_window()` |
| Trigger close animation | `handlers/layer_shell.rs::layer_destroyed()` | `xdg_shell.rs::start_close_animation_for_window()` |
| Store animation state | `layer/mapped.rs::MappedLayer` | `layout/tile.rs::Tile` |
| Advance animations | `layer/mapped.rs` + `niri.rs` | `layout/tile.rs` + `layout/mod.rs` |
| Render animation | `layer/mapped.rs::render_normal()` | `layout/tile.rs::render()` |
| Cleanup on complete | `layer/mapped.rs` | `layout/scrolling.rs` closing handling |

## Feature Dependencies

```
Layer Animation Feature
├── [EXISTS] LayerAnimation struct (render logic)
├── [EXISTS] LayerOpenAnim/LayerCloseAnim config
├── [EXISTS] Animation core (timing, physics)
├── [EXISTS] Shader infrastructure (ProgramType::Open/Close)
├── [NEEDS] Animation triggers (map/unmap callbacks)
├── [NEEDS] State storage in MappedLayer
├── [NEEDS] Animation advancement in render loop
├── [NEEDS] Render integration
└── [NEEDS] Completion cleanup
```

## MVP Recommendation

For a minimal viable layer-shell animation implementation:

1. **Phase 1: Basic trigger and state** (medium effort)
   - Add animation field to `MappedLayer`
   - Trigger on surface map in `layer_shell.rs`
   - Trigger on surface destroy in `layer_shell.rs`

2. **Phase 2: Render integration** (medium effort)
   - Call animation render in `MappedLayer::render_normal()`
   - Handle animation advancement

3. **Phase 3: Completion and polish** (low effort)
   - Cleanup after animation completes
   - Test with various layer surface types (waybar, dunst, rofi)

## Confidence Assessment

| Area | Level | Reason |
|------|-------|--------|
| What's implemented | HIGH | Code reviewed in `layer_animation.rs`, `animations.rs` |
| Config structure | HIGH | Full `LayerOpenAnim`/`LayerCloseAnim` with defaults |
| What's missing | HIGH | Clear from grep patterns - no trigger calls exist |
| Integration approach | HIGH | Pattern identical to window animations |
| Shader support | HIGH | Already supports `ProgramType::Open`/`Close` |

## Sources

- `src/layer/layer_animation.rs` - Animation struct (181 lines)
- `niri-config/src/animations.rs` - Config definitions (lines 207-246)
- `src/layout/opening_window.rs` - Reference implementation (143 lines)
- `src/layout/closing_window.rs` - Reference implementation (~250 lines)
- `src/handlers/layer_shell.rs` - Where triggers needed
- `src/layer/mapped.rs` - Where state storage needed
- `src/niri.rs` - Where advancement called
