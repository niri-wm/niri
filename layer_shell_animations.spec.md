
# Layer-Shell Surface Animation Specification

A comprehensive design document for implementing animations for Wayland layer-shell surfaces in Niri, achieving **full parity with window open/close animations**.

**Note**: Unlike windows, layer surfaces do NOT resize, so there is no layer resize animation.

---

## Table of Contents

1. [Executive Summary](#executive-summary)
2. [Motivation](#motivation)
3. [Architecture Overview](#architecture-overview)
4. [Window Animation Reference](#window-animation-reference)
5. [Configuration Changes](#configuration-changes)
6. [Core Implementation](#core-implementation)
7. [Layer Open Animation (Parity)](#layer-open-animation-parity)
8. [Layer Close Animation (Parity)](#layer-close-animation-parity)
9. [Animation Approach](#animation-approach)
10. [Per-Surface Animation Rules](#per-surface-animation-rules)
11. [Rendering Integration](#rendering-integration)
12. [Shaders](#shaders)
13. [Edge Cases and Considerations](#edge-cases-and-considerations)
14. [Testing Strategy](#testing-strategy)
15. [Implementation Phases](#implementation-phases)
16. [Migration Path](#migration-path)
17. [Verification Checklist](#verification-checklist)

---

## Executive Summary

This specification defines the implementation of smooth, configurable animations for Wayland layer-shell surfaces in Niri, achieving **full parity with window open/close/resize animations**. Layer-shell surfaces include panels (waybar, polybar), notifications (dunst, swaync), launchers (rofi, wofi), and other overlay UI elements.

The implementation adds animation categories to Niri's existing animation system, supporting layer-shell surface open/close/resize animations triggered by layer-shell map/unmap/resize events, with the same features as window animations.

### Key Features (Full Parity)

- **Fade animations**: smooth opacity transitions for layer surfaces (same as window)
- **Position/slide animations**: slide in/out from edges using offset interpolation (NEW)
- **Scale animations**: scale transform for "popin" effects (NEW)
- **Custom shader support**: advanced animations via GLSL shaders (same as window)
- **Snapshot buffers**: render to offscreen texture before animation (same as window)
- **Spring physics**: optional spring-based animations with damping and stiffness
- **Global configuration**: layer-open, layer-close animation settings
- **Transaction blocking**: wait for pending transactions before close animation (same as window)
- **Zero breaking changes**: fully backward-compatible with existing configs

**Note**: Unlike window animations, layer surfaces do NOT have resize animations since layer surfaces are created with fixed sizes and never resize dynamically.

---

## Motivation

### Current State

Niri currently supports animations for:
- Window open/close/movement/resize (full implementation)
- Workspace switching
- UI elements (overview, screenshot, dialogs)
- **Layer-shell baba_is_float** (continuous floating animation only)

Layer-shell surfaces appear and disappear instantaneously with no open/close/resize animation support.

### Problem Statement

Users running layer-shell applications experience jarring UX when:
- Notifications (dunst) pop in without transition
- Panels (waybar) appear instantly on show
- Launchers (rofi) have no entry/exit animation
- Focus transitions between layers feel abrupt

### User Impact

This feature addresses requests from users migrating from Hyprland/Sway, where layer animations are standard. The lack of layer animations creates visual inconsistency in mixed desktop environments.

### Comparison with Hyprland

Hyprland implements layer animations via its `animations { ... layers { ... } }` config section with distinct "styles": `slide`, `popin`, `fade`. 

Niri takes a different approach:
- Uses the same unified `Animation` type as window animations
- Visual effects (fade, slide-like offset) are achieved through rendering interpolation
- Supports custom shaders for advanced effects
- Keeps the config system simple and consistent

This design choice prioritizes code simplicity and consistency with Niri's existing animation patterns over feature parity with Hyprland's config syntax.

---

## Architecture Overview

### System Components

```
┌─────────────────────────────────────────────────────────────────────┐
│                         Configuration Layer                          │
├─────────────────────────────────────────────────────────────────────┤
│  niri-config/src/animations.rs                                      │
│    ├── Animations struct                                            │
│    │   ├── layer_open (EXISTING - LayerOpenAnim)                   │
│    │   └── layer_close (EXISTING - LayerCloseAnim)                  │
│    └── LayerRule struct                                             │
│        └── baba_is_float (EXISTING)                                 │
├─────────────────────────────────────────────────────────────────────┤
│                      Animation Core                                  │
├─────────────────────────────────────────────────────────────────────┤
│  src/animation/mod.rs                                               │
│    └── Animation, Curve, Kind, Spring (REUSE existing types)         │
├─────────────────────────────────────────────────────────────────────┤
│                   Layer-Shell Handler                                │
├─────────────────────────────────────────────────────────────────────┤
│  src/handlers/layer_shell.rs                                        │
│    ├── layer_shell_handle_commit() ──► trigger open animation       │
│    │                   (currently: creates MappedLayer, no animation)│
│    └── layer_destroyed() ──► trigger close animation                │
│                    (currently: instant remove, no animation)         │
├─────────────────────────────────────────────────────────────────────┤
│                     MappedLayer                                       │
├─────────────────────────────────────────────────────────────────────┤
│  src/layer/mapped.rs                                                │
│    ├── bob_offset() (EXISTING - for baba_is_float)                  │
│    ├── are_animations_ongoing() (EXISTING - checks baba_is_float)   │
│    └── render_normal() (EXISTING - uses opacity + bob_offset)       │
│                                                                     │
│    ANIMATION STATE TO ADD:                                          │
│    ├── open_animation: Option<Animation> (NEW)                      │
│    ├── close_animation: Option<Animation> (NEW)                    │
│    ├── animated_alpha(): f64 (NEW)                                  │
│    ├── animated_offset(): Point<f64, Logical> (NEW - for slide)     │
│    └── animated_scale(): f64 (NEW - for popin)                      │
├─────────────────────────────────────────────────────────────────────┤
│                      Rendering                                       │
├─────────────────────────────────────────────────────────────────────┤
│  src/niri.rs                                                        │
│    └── are_animations_ongoing() check for layers (needs update)     │
└─────────────────────────────────────────────────────────────────────┘
```

### Existing: baba_is_float Animation

The codebase already has a layer animation mechanism: `baba_is_float`. This is a **continuous floating animation** that makes layers "bob" up and down using a sine wave:

```rust
// src/utils/mod.rs
pub fn baba_is_float_offset(now: Duration, view_height: f64) -> f64 {
    let now = now.as_secs_f64();
    let amplitude = view_height / 96.;
    amplitude * ((f64::consts::TAU * now / 3.6).sin() - 1.)
}
```

This is triggered via layer-rules and works continuously. The new open/close animations are **different** - they are triggered on map/unmap events and run once to completion.

### Animation Flow

```
Layer Surface Events                    Animation Lifecycle
┌─────────────────────┐                 ┌─────────────────────────┐
│ layer_shell.map     │ ─────────────► │ take_snapshot()         │
│                     │                 │ create_animation()      │
│                     │                 │                         │
│ layer_shell.commit  │ ─────────────► │ animated_render()       │
│                     │                 │ (interpolate values)    │
│                     │                 │                         │
│ layer_shell.unmap   │ ─────────────► │ animation_done()        │
└─────────────────────┘                 └─────────────────────────┘
```

### State Machine

```
┌─────────────┐    commit    ┌─────────────────┐    done     ┌──────────┐
│   PENDING   │ ───────────► │    ANIMATING    │ ──────────► │  FINAL   │
│  (not yet   │              │  (interpolating)│              │  (shown) │
│   shown)    │              └─────────────────┘              └──────────┘
└─────────────┘

┌─────────────┐    unmap     ┌─────────────────┐    done     ┌──────────┐
│   FINAL     │ ───────────► │    CLOSING      │ ──────────► │  HIDDEN  │
│  (shown)    │              │  (animating out)│              │  (gone)  │
└─────────────┘              └─────────────────┘              └──────────┘
```

---

## Window Animation Reference

This section documents the full window animation implementation to ensure layer animations achieve complete parity. All layer animations should follow the same patterns.

### Window Open Animation

**File**: `src/layout/opening_window.rs`

```rust
pub struct OpenAnimation {
    anim: Animation,                    // 0→1 interpolation
    random_seed: f32,                   // For shader effects
    buffer: OffscreenBuffer,           // Snapshot of window contents
}
```

**Trigger Points**:
- `src/handlers/compositor.rs:235` - `start_open_animation_for_window()` on window map
- `src/layout/mod.rs:4585` - `Layout::start_open_animation_for_window()`

**Rendering**:
- Renders window to offscreen texture first (snapshot buffer)
- If custom shader: applies shader with progress uniforms
- Default: fades alpha + scales from center: `(progress / 2. + 0.5).max(0.)`

### Window Close Animation

**File**: `src/layout/closing_window.rs`

```rust
pub struct ClosingWindow {
    buffer: TextureBuffer,              // Snapshot of window contents
    blocked_out_buffer: TextureBuffer,  // For transaction blocking
    anim_state: AnimationState,
    // AnimationState::Waiting { blocker, anim } | AnimationState::Animating(anim)
}
```

**Key Feature**: Transaction-aware - waits for pending transactions before animating (via `TransactionBlocker`)

**Trigger Points**:
- `src/handlers/compositor.rs:276` - on window unmap
- `src/layout/mod.rs:4661` - `Layout::start_close_animation_for_window()`

**Rendering**:
- Renders window to texture on creation
- If custom shader: applies shader with progress
- Default: fades out + shrinks toward center

**Key Feature**: Transaction-aware - waits for pending transactions before animating (via `TransactionBlocker`)

**Trigger Points**:
- `src/handlers/compositor.rs:276` - on window unmap
- `src/layout/mod.rs:4661` - `Layout::start_close_animation_for_window()`

**Rendering**:
- Renders window to texture on creation
- If custom shader: applies shader with progress
- Default: fades out + shrinks toward center

### Window Resize Animation

**File**: `src/layout/tile.rs`

```rust
struct ResizeAnimation {
    anim: Animation,                          // 0→1 interpolation
    size_from: Size<f64, Logical>,            // Previous window size
    snapshot: LayoutElementRenderSnapshot,   // Render snapshot before resize
    offscreen: OffscreenBuffer,               // For intermediate rendering
    tile_size_from: Size<f64, Logical>,      // Previous tile size
    fullscreen_progress: Option<Animation>,  // For fullscreen transition
    expanded_progress: Option<Animation>,    // For maximize transition
}
```

**Creation Threshold**:
```rust
const RESIZE_ANIMATION_THRESHOLD: f64 = 10.0; // pixels
let change = max(|(new_size - old_size).x|, |(new_size - old_size).y|);
if change > RESIZE_ANIMATION_THRESHOLD {
    // Start animation
}
```

### Animation Render Pipeline

**Animation Advancement** (`src/layout/mod.rs:2540`):
```
Niri::advance_animations()
  → Layout::advance_animations()
      → Workspace::advance_animations()
          → ScrollingSpace::advance_animations()
              → Tile::advance_animations()
                  → Clears completed animations
```

### Custom Shader Uniforms

All window animations provide these uniforms to custom shaders:

```glsl
uniform float niri_progress;         // Raw 0→1
uniform float niri_clamped_progress; // Capped at 1.0
uniform float niri_random_seed;      // For procedural effects
uniform mat3 niri_input_to_geo;     // Coordinate transforms
uniform vec2 niri_geo_size;
uniform mat3 niri_geo_to_tex;
uniform sampler2D niri_tex;
```

### Animation Lifecycle Pattern

```
1. TRIGGER
   ├── Layer maps → start_open_animation()
   └── Layer unmaps → start_close_animation()

2. STATE
   ├── Animation struct created with clock, from/to, config
   ├── Stored in MappedLayer (open_animation, close_animation)
   └── Snapshot buffer captured (offscreen render)

3. ADVANCE (every frame)
   ├── Animation::value() queries clock
   ├── is_done() checks completion
   └── Completed animations cleared

4. RENDER
   ├── Check custom shader availability
   ├── Interpolate values (alpha, scale, offset)
   ├── Render to offscreen if needed
   └── Composite to output
```

**Note**: Layer surfaces do NOT resize, so no resize animation is needed.

### Key Implementation Details for Parity

1. **Snapshot Buffers**: Always render layer to offscreen texture before animating
2. **Transaction Blocking**: For close animations, wait for pending transactions
3. **Resize Threshold**: Only animate when size change exceeds 10px
4. **Animation State**: Store `Option<Animation>` in MappedLayer
5. **Clock**: Reuse existing `clock: Clock` field in MappedLayer
6. **Custom Shaders**: Support same shader interface as windows
7. **Progress Interpolation**: Use `anim.value()` and `anim.clamped_value()`

---

## Configuration Changes

### Current State (Already Implemented)

The config for layer animations already exists in `niri-config/src/animations.rs`:

```rust
// niri-config/src/animations.rs (EXISTING)
#[derive(Debug, Clone, PartialEq)]
pub struct LayerOpenAnim {
    pub anim: Animation,
    pub custom_shader: Option<String>,
}

impl Default for LayerOpenAnim {
    fn default() -> Self {
        Self {
            anim: Animation {
                off: false,
                kind: Kind::Easing(EasingParams {
                    duration_ms: 200,
                    curve: Curve::EaseOutExpo,
                }),
            },
            custom_shader: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayerCloseAnim {
    pub anim: Animation,
    pub custom_shader: Option<String>,
}

impl Default for LayerCloseAnim {
    fn default() -> Self {
        Self {
            anim: Animation {
                off: false,
                kind: Kind::Easing(EasingParams {
                    duration_ms: 150,
                    curve: Curve::EaseOutQuad,
                }),
            },
            custom_shader: None,
        }
    }
}
```

And in `Animations` struct:
```rust
pub struct Animations {
    // ... existing window animations ...
    pub layer_open: LayerOpenAnim,     // EXISTING
    pub layer_close: LayerCloseAnim,   // EXISTING
    // Layer surfaces don't resize, so no layer_resize needed
    // ...
}
```

**Note**: Layer surfaces are created with fixed dimensions and never resize dynamically, so no `layer_resize` animation is needed. This is different from windows which can be resized.

### Layer Rules - baba_is_float (Already Implemented)

The existing `baba_is_float` option in layer rules:

```rust
// niri-config/src/layer_rule.rs (EXISTING)
#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq)]
pub struct LayerRule {
    // ... existing fields ...
    #[knuffel(child, unwrap(argument))]
    pub baba_is_float: Option<bool>,
}
```

This triggers a continuous floating animation, NOT an open/close animation.

### Animation Config Schema (What's Needed)

Add animation types to `niri-config/src/animations.rs` following the existing pattern for window animations - **these already exist**, no changes needed:

```rust
/// Animation configuration for layer-shell surface opening.
/// Follows the same pattern as WindowOpenAnim.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerOpenAnim {
    pub anim: Animation,
    pub custom_shader: Option<String>,
}

/// Animation configuration for layer-shell surface closing.
/// Follows the same pattern as WindowCloseAnim.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerCloseAnim {
    pub anim: Animation,
    pub custom_shader: Option<String>,
}
```

**Note on Animation Styles**: Unlike Hyprland which has animation "styles" (slide, fade, popin), Niri's animation system uses a unified `Animation` type with `kind: Kind` (either `Easing` or `Spring`). The visual effect (slide, fade, etc.) is determined by how the animation values are applied during rendering, not by a separate style enum. This keeps the animation system simple and consistent with existing window animations.

### Animations Struct Integration (Already Done)

The animations struct already has layer_open and layer_close - no changes needed:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct Animations {
    // ... existing window animations ...

    /// Layer-shell surface animations (EXISTING).
    pub layer_open: LayerOpenAnim,
    pub layer_close: LayerCloseAnim,
}
```

### Default Configuration

```kdl
// Default animations configuration
animations {
    // ... existing window animations ...

    // Layer-shell animations (EXISTING - already defined)
    layer-open {
        duration-ms 200
        curve "ease-out-expo"
    }
    layer-close {
        duration-ms 150
        curve "ease-out-quad"
    }
}
```

**Note**: Custom shaders can be added for layer animations just like window animations:
```kdl
layer-open {
    duration-ms 300
    curve "ease-out-expo"
    custom-shader "path/to/shader.glsl"
}
```

### Per-Surface Animation Rules (DEFERRED)

**Status**: Animation overrides through layer-rules are deferred to a future phase. The initial implementation will use global layer animation settings only.

**Rationale**: 
- Layer-rule animation overrides add significant complexity to the config system
- The existing `LayerRule` / `ResolvedLayerRules` pattern works best for simple property overrides
- Animation configuration is complex (duration, curve/spring params, custom shaders)
- Global settings provide 80% of the value with 20% of the complexity

**Future Architecture** (when implemented):
Animation overrides through layer-rules would follow the existing pattern:

```rust
// niri-config/src/layer_rule.rs
#[derive(knuffel::Decode, Debug, Clone, PartialEq, Default)]
pub struct LayerAnimationRule {
    /// Override for open animation (merges with global default).
    #[knuffel(child)]
    pub open: Option<LayerOpenAnimOverride>,
    
    /// Override for close animation (merges with global default).
    #[knuffel(child)]
    pub close: Option<LayerCloseAnimOverride>,
    
    /// Completely disable animations for matching surfaces.
    #[knuffel(child)]
    pub animation_off: bool,
}

#[derive(knuffel::Decode, Debug, Clone, PartialEq, Default)]
pub struct LayerRule {
    // ... existing fields ...
    
    /// Animation configuration (FUTURE).
    #[knuffel(child, default)]
    pub animation: LayerAnimationRule,
}

// src/layer/mod.rs
#[derive(Debug, Default, PartialEq)]
pub struct ResolvedLayerRules {
    // ... existing fields ...
    
    /// Animation overrides (FUTURE).
    pub animation: LayerAnimationRule,
}
```

**Deferred Layer Rule Examples** (for future reference):

```kdl
// Future: Disable animations for specific surface
layer-rule {
    match namespace="rofi"
    animation-off
}

// Future: Custom animation for specific surface
layer-rule {
    match namespace="dunst"
    animation {
        open {
            duration-ms 150
            curve "ease-out-back"
        }
    }
}
```

---

## Core Implementation

### Animation Trigger Location

Animations are triggered in the layer shell handler, similar to how window animations work. Looking at `src/handlers/layer_shell.rs`, animations should be triggered in `layer_shell_handle_commit()`:

```rust
// In src/handlers/layer_shell.rs, in layer_shell_handle_commit()

// When a layer surface maps (becomes visible)
if was_unmapped {
    // ... existing code to create MappedLayer ...
    
    // Start open animation after creating MappedLayer
    let config = self.niri.config.borrow();
    if let Some(mapped) = self.niri.mapped_layer_surfaces.get(layer) {
        mapped.borrow_mut().start_open_animation(&config.animations.layer_open);
    }
}

// When a layer surface unmaps (becomes hidden)
// This happens in layer_destroyed() - need to track previous state
```

### Animation State in MappedLayer

Add animation state to `src/layer/mapped.rs` following Niri's existing animation patterns (full window parity for open/close):

```rust
use crate::animation::Animation;
use crate::render_helpers::TextureBuffer;

/// Add these fields to the existing MappedLayer struct.
pub struct MappedLayer {
    // ... existing fields (surface, rules, shadow, clock, etc.) ...

    // === ANIMATION STATE (NEW - Full Window Parity for Open/Close) ===

    /// Snapshot buffer for open animation (renders layer to offscreen texture).
    open_buffer: Option<TextureBuffer>,

    /// Animation for layer opening (alpha 0.0 -> 1.0, scale, offset).
    open_animation: Option<Animation>,

    /// Snapshot buffer for close animation.
    close_buffer: Option<TextureBuffer>,

    /// Animation for layer closing (alpha 1.0 -> 0.0, scale, offset).
    close_animation: Option<Animation>,
}
```

The existing `clock: Clock` field in `MappedLayer` is reused for driving these animations.

**Note**: Layer surfaces don't resize, so no resize animation state is needed.

### Animation Methods

Add animation methods to `MappedLayer` following Niri's patterns (full window parity):

```rust
impl MappedLayer {
    /// Start open animation with given configuration.
    /// Creates an animation that interpolates alpha from 0.0 to 1.0.
    /// Captures snapshot buffer for rendering.
    pub fn start_open_animation(&mut self, config: &LayerOpenAnim) {
        // Skip if animations are disabled
        if config.anim.off {
            return;
        }

        // Capture snapshot buffer (same as window open animation)
        self.open_buffer = self.capture_snapshot_buffer();

        // Create animation from 0.0 to 1.0 (transparent to opaque)
        self.open_animation = Some(Animation::new(
            self.clock.clone(),
            0.0,  // start: fully transparent
            1.0,  // end: fully opaque
            0.0,  // initial velocity
            config.anim,
        ));
    }

    /// Start close animation with given configuration.
    /// Creates an animation that interpolates alpha from 1.0 to 0.0.
    /// Captures snapshot buffer (same as window close animation).
    pub fn start_close_animation(&mut self, config: &LayerCloseAnim) {
        // Skip if animations are disabled
        if config.anim.off {
            return;
        }

        // Capture snapshot buffer before closing
        self.close_buffer = self.capture_snapshot_buffer();

        // Create animation from 1.0 to 0.0 (opaque to transparent)
        self.close_animation = Some(Animation::new(
            self.clock.clone(),
            1.0,  // start: fully opaque
            0.0,  // end: fully transparent
            0.0,  // initial velocity
            config.anim,
        ));
    }

    /// Capture snapshot buffer for animation (same as window).
    fn capture_snapshot_buffer(&self) -> Option<TextureBuffer> {
        // Render current layer state to offscreen texture
        // Implementation follows OpenAnimation::render() pattern
    }

    /// Get current animated alpha (0.0 to 1.0).
    /// Returns the animation progress, or 1.0 if not animating.
    pub fn animated_alpha(&self) -> f64 {
        if let Some(ref anim) = self.open_animation {
            return anim.clamped_value();
        }
        if let Some(ref anim) = self.close_animation {
            return anim.clamped_value();
        }
        1.0 // Default to fully visible when not animating
    }

    /// Get animated offset for slide effect.
    /// Interpolates from edge position to final position.
    pub fn animated_offset(&self, layer_anchor: Anchor) -> Point<f64, Logical> {
        let progress = self.open_animation
            .map(|a| a.clamped_value())
            .or_else(|| self.close_animation.map(|a| a.clamped_value()))
            .unwrap_or(1.0);

        // Calculate slide offset based on layer anchor
        // e.g., for bottom anchor: offset = (0, height * (1 - progress))
        match layer_anchor {
            Anchor::Bottom => Point::new(0.0, self.size.h * (1.0 - progress)),
            Anchor::Top => Point::new(0.0, -self.size.h * (1.0 - progress)),
            Anchor::Left => Point::new(-self.size.w * (1.0 - progress), 0.0),
            Anchor::Right => Point::new(self.size.w * (1.0 - progress), 0.0),
            _ => Point::default(),
        }
    }

    /// Get animated scale for popin effect.
    /// Interpolates from scale 0.5 to 1.0.
    pub fn animated_scale(&self) -> f64 {
        let progress = self.open_animation
            .map(|a| a.clamped_value())
            .or_else(|| self.close_animation.map(|a| a.clamped_value()))
            .unwrap_or(1.0);

        // Default: scale from 0.5 to 1.0
        progress * 0.5 + 0.5
    }

    /// Check if any animation is ongoing.
    pub fn are_animations_ongoing(&self) -> bool {
        // Check existing baba_is_float animation
        if self.rules.baba_is_float {
            return true;
        }
        // Check layer open/close animations
        self.open_animation.as_ref().map_or(false, |a| !a.is_done())
            || self.close_animation.as_ref().map_or(false, |a| !a.is_done())
    }

    /// Advance animations and clear completed ones.
    pub fn advance_animations(&mut self) {
        if let Some(ref mut anim) = self.open_animation {
            if anim.is_done() {
                self.open_animation = None;
                self.open_buffer = None;
            }
        }
        if let Some(ref mut anim) = self.close_animation {
            if anim.is_done() {
                self.close_animation = None;
                self.close_buffer = None;
            }
        }
    }

    /// Clear all animation state.
    pub fn clear_animation_state(&mut self) {
        self.open_animation = None;
        self.open_buffer = None;
        self.close_animation = None;
        self.close_buffer = None;
    }
}
```

### Layer Shell Handler Integration

Update `src/handlers/layer_shell.rs` to trigger animations:

```rust
impl WlrLayerShellHandler for State {
    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        wl_output: Option<WlOutput>,
        layer: Layer,
        namespace: String,
    ) {
        // ... existing code ...

        // NEW: Check if this surface should animate on open
        if let Some(output) = output {
            let config = self.niri.config.borrow();
            let layer_anim_config = config.animations.layer.open.clone();

            // Check for per-surface animation rules
            let resolved = ResolvedLayerRules::compute(
                &config.layer_rules,
                &layer_surface,
                self.niri.is_at_startup,
            );

            // Apply animation if not disabled
            if !resolved.animation.disabled {
                // Store animation config to apply after surface is mapped
                // This will be triggered on first commit
                if let Some(mapped) = self.niri.mapped_layer_surfaces.get(&layer) {
                    mapped.borrow_mut().start_open_animation(
                        &layer_anim_config.unwrap_or_default(),
                        self.niri.clock.clone(),
                    );
                }
            }
        }
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        // NEW: Trigger close animation before unmap
        if let Some((output, mut map, layer)) = /* find mapped layer */ {
            let config = self.niri.config.borrow();
            let layer_anim_config = config.animations.layer.close.clone();

            // Start close animation
            if let Some(mapped) = self.niri.mapped_layer_surfaces.get(&layer) {
                mapped.borrow_mut().start_close_animation(
                    &layer_anim_config.unwrap_or_default(),
                    self.niri.clock.clone(),
                );
            }
        }

        // ... existing unmap code ...
    }
}
```

### Animation Progress Tracking

Update `src/niri.rs` to include layer animations in progress tracking:

```rust
fn render(&mut self, backend: &impl Backend, output: &Output) {
    // ... existing render setup ...

    // NEW: Check layer animations
    let mut unfinished_animations_remain = false;

    for (_, mapped_layer) in &self.mapped_layer_surfaces {
        if mapped_layer.are_animations_ongoing() {
            unfinished_animations_remain = true;
        }
    }

    // Update animation frame callback logic
    if unfinished_animations_remain {
        // Ensure frame callbacks are sent for smooth animation
        self.send_frame_callbacks(output);
    }
}
```

### Niri::advance_animations Integration

Update `src/niri.rs` to advance layer animations alongside other animations:

```rust
pub fn advance_animations(&mut self) {
    let _span = tracy_client::span!("Niri::advance_animations");

    self.layout.advance_animations();
    self.config_error_notification.advance_animations();
    self.exit_confirm_dialog.advance_animations();
    self.screenshot_ui.advance_animations();
    self.window_mru_ui.advance_animations();

    // NEW: Advance layer animations
    for (_, mapped_layer) in &mut self.mapped_layer_surfaces {
        mapped_layer.advance_animations();
    }

    for state in self.output_state.values_mut() {
        if let Some(transition) = &mut state.screen_transition {
            if transition.is_done() {
                state.screen_transition = None;
            }
        }
    }
}
```

---

## Layer Open Animation (Parity)

This section details the layer open animation implementation to achieve full parity with window open animations.

### Trigger Points

| Location | Trigger |
|----------|---------|
| `src/handlers/layer_shell.rs` | When layer surface maps (becomes visible) |
| `src/layer/mapped.rs` | `MappedLayer::start_open_animation()` |

### Implementation Pattern

Following `src/layout/opening_window.rs`:

```rust
/// Layer open animation state
pub struct LayerOpenAnimation {
    anim: Animation,              // 0→1 interpolation
    random_seed: f32,             // For shader effects
    buffer: Option<TextureBuffer>, // Snapshot of layer contents
}
```

### Rendering

1. **Capture snapshot**: Render layer to offscreen texture before first frame
2. **Check custom shader**: If `custom_shader` configured in `LayerOpenAnim`, use shader
3. **Default rendering** (no custom shader):
   - Alpha: `clamped_progress * opacity`
   - Scale: `(progress / 2. + 0.5).max(0.)` - scale from center
   - Offset: Apply slide offset based on layer anchor (for slide effect)

### Configuration (from `niri-config/src/animations.rs`)

```rust
impl Default for LayerOpenAnim {
    fn default() -> Self {
        Self {
            anim: Animation {
                off: false,
                kind: Kind::Easing(EasingParams {
                    duration_ms: 200,
                    curve: Curve::EaseOutExpo,
                }),
            },
            custom_shader: None,
        }
    }
}
```

### Visual Effects (via Rendering Interpolation)

- **Fade**: Alpha interpolates 0.0 → 1.0
- **Slide**: Offset interpolates from edge position to final position based on `Anchor`
- **Scale**: Scale interpolates 0.5 → 1.0 for "popin" effect

---

## Layer Close Animation (Parity)

This section details the layer close animation implementation to achieve full parity with window close animations.

### Trigger Points

| Location | Trigger |
|----------|---------|
| `src/handlers/layer_shell.rs` | When layer surface unmaps (becomes hidden) |
| `src/layer/mapped.rs` | `MappedLayer::start_close_animation()` |

### Implementation Pattern

Following `src/layout/closing_window.rs`:

```rust
/// Layer close animation state
pub struct LayerCloseAnimation {
    buffer: Option<TextureBuffer>,  // Snapshot of layer contents
    anim: Animation,               // 1.0→0.0 interpolation
}
```

### Key Feature: Transaction Blocking

Like window close animations, layer close animations should wait for pending transactions:

```rust
enum AnimationState {
    Waiting { blocker: TransactionBlocker, anim: Animation },
    Animating(Animation),
}
```

However, layer surfaces typically don't have pending transactions like windows, so this may be simplified.

### Rendering

1. **Capture snapshot**: Render layer to offscreen texture when close starts
2. **Check custom shader**: If `custom_shader` configured in `LayerCloseAnim`, use shader
3. **Default rendering** (no custom shader):
   - Alpha: `(1.0 - clamped_progress) * opacity`
   - Scale: `((1.0 - clamped_progress) / 5. + 0.8)` - shrink toward center
   - Offset: Apply slide offset based on layer anchor (reverse of open)

### Configuration (from `niri-config/src/animations.rs`)

```rust
impl Default for LayerCloseAnim {
    fn default() -> Self {
        Self {
            anim: Animation {
                off: false,
                kind: Kind::Easing(EasingParams {
                    duration_ms: 150,
                    curve: Curve::EaseOutQuad,
                }),
            },
            custom_shader: None,
        }
    }
}
```

---

## Animation Approach

### Niri's Animation Model

Unlike Hyprland which has distinct animation "styles" (slide, fade, popin), Niri uses a simpler, more unified model:

1. **Animation Type**: All animations use the same `Animation` struct with:
   - `off: bool` - to disable the animation
   - `kind: Kind` - either `Easing(EasingParams)` or `Spring(SpringParams)`

2. **Visual Effect**: The visual effect (sliding, fading, scaling) is determined by how the animation values are applied during rendering, not by a separate style enum.

3. **Custom Shaders**: Like window animations, layer animations support custom shaders for advanced effects.

### Configuration Examples

**Basic easing animation**:
```kdl
animations {
    layer-open {
        duration-ms 200
        curve "ease-out-expo"
    }
}
```

**Spring physics animation**:
```kdl
animations {
    layer-open {
        spring {
            damping-ratio 0.8
            stiffness 1000
            epsilon 0.001
        }
    }
}
```

**With custom shader**:
```kdl
animations {
    layer-open {
        duration-ms 300
        curve "ease-out-expo"
        custom-shader "~/.config/niri/shaders/layer-open.glsl"
    }
}
```

**Disable animations**:
```kdl
animations {
    layer-open { off }
}
```

### Implementation Approach

The rendering system will interpolate layer surface properties based on the animation progress:

1. **Alpha (opacity)**: Interpolate from 0.0 to 1.0 during open, 1.0 to 0.0 during close
2. **Position**: Apply an offset based on the layer's configured position (e.g., slide from edge)
3. **Scale**: Optionally apply scale transforms for "popin" effects

This keeps the animation system consistent with existing window animations while allowing visual effects similar to Hyprland's styles through rendering interpolation.

---

## Per-Surface Animation Rules

### Rule Matching (Deferred)

**Note**: Per-surface animation rules through layer-rules are deferred. The initial implementation uses global animation settings only.

When implemented in a future phase, animation rules would follow the same matching logic as existing layer rules:

```rust
// FUTURE: When layer-rule animation overrides are implemented
impl ResolvedLayerRules {
    pub fn compute(rules: &[LayerRule], surface: &LayerSurface, is_at_startup: bool) -> Self {
        let mut resolved = ResolvedLayerRules::default();

        for rule in rules {
            // ... existing matching logic ...

            // FUTURE: Apply animation rules
            // if rule.animation_off {
            //     resolved.animation_off = true;
            // }
            // if let Some(ref anim) = rule.animation_override {
            //     resolved.animation_override = Some(anim.clone());
            // }
        }

        resolved
    }
}
```

### Global Configuration Only (Initial Implementation)

For the initial implementation, all layer surfaces use the global animation settings:

```kdl
animations {
    // All layer surfaces will use these settings
    layer-open {
        duration-ms 200
        curve "ease-out-expo"
    }
    layer-close {
        duration-ms 150
        curve "ease-out-quad"
    }
}
```

To disable animations for specific applications, users can:
1. Disable animations globally: `layer-open { off }`
2. Or use application-level settings if the layer-shell app supports it

---

## Shaders

Layer animations support custom GLSL shaders for advanced effects, achieving full parity with window animation shaders.

### Shader Location

Custom shaders for layer animations follow the same pattern as window animations:

```
src/render_helpers/shaders/
├── layer_open_prelude.frag   # Layer open shader (entry)
├── layer_open_epilogue.frag  # Layer open shader (exit)
├── layer_close_prelude.frag # Layer close shader (entry)
├── layer_close_epilogue.frag# Layer close shader (exit)
```

### Shader Uniforms

All layer animations provide these uniforms to custom shaders (same as window):

```glsl
uniform float niri_progress;         // Raw 0→1
uniform float niri_clamped_progress; // Capped at 1.0
uniform float niri_random_seed;      // For procedural effects
uniform mat3 niri_input_to_geo;     // Coordinate transforms
uniform vec2 niri_geo_size;
uniform mat3 niri_geo_to_tex;
uniform sampler2D niri_tex;
```

### Configuration

```kdl
animations {
    layer-open {
        duration-ms 300
        curve "ease-out-expo"
        custom-shader "~/.config/niri/shaders/layer-open.glsl"
    }
    layer-close {
        duration-ms 200
        curve "ease-out-quad"
        custom-shader "~/.config/niri/shaders/layer-close.glsl"
    }
}
```

### Shader Implementation Pattern

Follow the existing window shader pattern in `src/render_helpers/shaders/`:

```glsl
#version 450

// Progress uniforms (provided by niri)
uniform float niri_progress;
uniform float niri_clamped_progress;
uniform float niri_random_seed;
uniform mat3 niri_input_to_geo;
uniform vec2 niri_geo_size;
uniform mat3 niri_geo_to_tex;
uniform sampler2D niri_tex;

// Your custom effect here
void main() {
    vec4 color = texture(niri_tex, gl_TexCoord[0].xy);
    
    // Example: fade effect
    float alpha = niri_clamped_progress;
    
    // Example: slide offset
    // vec2 offset = vec2(0.0, niri_geo_size.y * (1.0 - niri_clamped_progress));
    
    gl_FragColor = vec4(color.rgb, color.a * alpha);
}
```

### Shader Functions to Add

Add these functions to `src/render_helpers/shaders/mod.rs` following the window shader pattern:

```rust
// In src/render_helpers/shaders/mod.rs

/// Set custom layer open animation shader.
pub fn set_custom_layer_open_program(renderer: &mut GlesRenderer, src: Option<&str>) {
    let program = if let Some(src) = src {
        match compile_layer_open_program(renderer, src) {
            Ok(program) => Some(program),
            Err(err) => {
                warn!("error compiling custom layer open shader: {err:?}");
                return;
            }
        }
    } else {
        None
    };

    if let Some(prev) = Shaders::get(renderer).replace_custom_layer_open_program(program) {
        if let Err(err) = prev.destroy(renderer) {
            warn!("error destroying previous custom layer open shader: {err:?}");
        }
    }
}

/// Set custom layer close animation shader.
pub fn set_custom_layer_close_program(renderer: &mut GlesRenderer, src: Option<&str>) {
    // Similar to set_custom_layer_open_program
}
```

### Config Reload Integration

Update `src/niri.rs` to reload custom layer shaders when config changes:

```rust
// In src/niri.rs, where custom shaders are reloaded

if config.animations.layer_open.custom_shader
    != old_config.animations.layer_open.custom_shader
{
    let src = config.animations.layer_open.custom_shader.as_deref();
    self.backend.with_primary_renderer(|renderer| {
        shaders::set_custom_layer_open_program(renderer, src);
    });
    shaders_changed = true;
}

if config.animations.layer_close.custom_shader
    != old_config.animations.layer_close.custom_shader
{
    let src = config.animations.layer_close.custom_shader.as_deref();
    self.backend.with_primary_renderer(|renderer| {
        shaders::set_custom_layer_close_program(renderer, src);
    });
    shaders_changed = true;
}
```

---

## Rendering Integration

```rust
pub fn render_normal<R: NiriRenderer>(
    &self,
    renderer: &mut R,
    location: Point<f64, Logical>,
    target: RenderTarget,
    push: &mut dyn FnMut(LayerSurfaceRenderElement<R>),
) {
    let scale = Scale::from(self.scale);

    // Get animated alpha (0.0 to 1.0) or use default opacity
    let base_alpha = self.rules.opacity.unwrap_or(1.);
    let animated_alpha = self.animated_alpha() * base_alpha;
    let location = location + self.bob_offset();

    if target.should_block_out(self.rules.block_out_from) {
        // Render blocked-out buffer
        let location = location.to_physical_precise_round(scale).to_logical(scale);
        let elem = SolidColorRenderElement::from_buffer(
            &self.block_out_buffer,
            location,
            animated_alpha,
            Kind::Unspecified,
        );
        push(elem.into());
    } else {
        // Render surface with animated alpha
        let surface = self.surface.wl_surface();
        push_elements_from_surface_tree(
            renderer,
            surface,
            location.to_physical_physical_round(scale),
            scale,
            animated_alpha,
            Kind::ScanoutCandidate,
            &mut |elem| push(elem.into()),
        );
    }

    // Render shadow
    let location = location.to_physical_precise_round(scale).to_logical(scale);
    self.shadow.render(renderer, location, &mut |elem| push(elem.into()));
}
```

**Implementation Details**:
- Initial implementation uses alpha interpolation only (fade effect)
- `animated_alpha()` returns 0.0→1.0 for open, 1.0→0.0 for close
- Alpha is applied via `push_elements_from_surface_tree()` 
- Uses existing `bob_offset()` for floating animation (like baba_is_float)
- Custom shaders are NOT supported in initial implementation (simpler than window animations)
- Future enhancements: slide offset, scale transforms

### Animation During Frame Callbacks

Ensure smooth animation by sending frame callbacks:

```rust
impl State {
    fn send_frame_callbacks(&mut self, output: &Output) {
        // NEW: Also check layer animations
        let needs_frame = self.mapped_layer_surfaces.values()
            .any(|l| l.borrow().are_animations_ongoing());

        if needs_frame {
            // Send frame callbacks to animated layers
            for (_, mapped) in &self.mapped_layer_surfaces {
                let layer = mapped.borrow();
                if layer.are_animations_ongoing() {
                    if let Some(surface) = layer.surface().wl_surface().clone() {
                        self.niri.queue_frame_callback(output, surface);
                    }
                }
            }
        }

        // ... existing window frame callback logic ...
    }
}
```

---

## Edge Cases and Considerations

### Animation Interruption

**Scenario**: User hides a layer while animation is in progress.

**Handling**: When switching from open to close animation (or vice versa), the current animation is simply replaced. The layer will jump to its current animated state and continue from there.

```rust
impl MappedLayer {
    /// Handle animation interruption by switching to the new animation type.
    pub fn handle_animation_interruption(&mut self, config: &LayerCloseAnim) {
        // Clear existing animation - layer jumps to current state
        self.open_animation = None;
        
        // Start close animation from current alpha
        if !config.anim.off {
            let current_alpha = self.animated_alpha();
            self.close_animation = Some(Animation::new(
                self.clock.clone(),
                current_alpha,  // start from current interpolated value
                0.0,            // end at fully transparent
                0.0,
                config.anim,
            ));
        }
    }
}
```

### Rapid Open/Close Cycles

**Scenario**: Application rapidly maps/unmaps surface (e.g., dunst notifications).

**Handling**: The animation system naturally handles rapid toggles - each new animation simply replaces the previous one. The layer will animate to its current state without complex coalescing logic.

For the initial implementation, no special handling is required. Future enhancements could add debouncing if needed.

### Multiple Outputs

**Scenario**: Layer surface spans multiple outputs or moves between outputs.

**Handling**:
- Animation state is per-MappedLayer instance
- When layer moves to new output, animation continues with current interpolated values
- The animation itself is independent of viewport size (interpolates 0.0->1.0)
- Visual offset calculations (if any) use the current output's viewport at render time

### Exclusive Zones

**Scenario**: Layer with exclusive zone changes visibility.

**Handling**:
- Track exclusive zone state changes via `layer.cached_state().exclusive_zone`
- Trigger open animation when layer enters exclusive zone (becomes visible)
- Trigger close animation when layer exits exclusive zone (becomes hidden)
- Ensure animation respects exclusive zone constraints (don't animate outside valid area)

```rust
impl MappedLayer {
    /// Check if exclusive zone state changed and trigger appropriate animation.
    pub fn check_exclusive_zone_change(
        &mut self,
        open_config: &LayerOpenAnim,
        close_config: &LayerCloseAnim,
    ) -> bool {
        let current_zone = self.surface.cached_state().exclusive_zone;
        let was_visible = self.exclusive_zone_visible;

        let is_visible = match current_zone {
            ExclusiveZone::DontCare => true,  // Not excluded
            ExclusiveZone::Exclusive(_) => false,  // Excluded
            ExclusiveZone::Neutral => true,   // Not excluded
        };

        if was_visible != is_visible {
            self.exclusive_zone_visible = is_visible;
            if is_visible {
                // Zone changed to visible - trigger open animation
                self.start_open_animation(open_config);
            } else {
                // Zone changed to hidden - trigger close animation
                self.start_close_animation(close_config);
            }
            return true;
        }
        false
    }
}
```

### Animations Disabled

**Scenario**: User sets `animations off` globally.

**Handling**:
```rust
impl MappedLayer {
    pub fn should_animate(&self, config: &LayerOpenAnim) -> bool {
        // Check global animation state
        !config.anim.off
    }
}
```

**Note**: Per-surface animation disable through layer-rules is deferred.

### Scale and HiDPI

**Scenario**: Animation coordinates must handle fractional scaling.

**Handling**:
- All animation values stored in logical coordinates
- Convert to physical for rendering
- Round to nearest pixel at render time

### Compositing Effects

**Scenario**: Layer with blur, shadows, or other effects.

**Handling**:
- Animation applies to the composited result
- Effects render at animated position
- No special handling required - effects compose normally

### Window Manager Focus

**Scenario**: Animating layer takes/gives focus.

**Handling**:
- Focus changes after animation completes
- Track "animating for focus" state separately
- Delay focus update until `is_done()` returns true

### Performance

**Scenario**: Many animated layers (e.g., notification storm).

**Handling**:
- Limit concurrent animations per layer type
- Skip animation for rapid successive events
- Consider animation coalescing

---

## Testing Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layer_animation_progress() {
        // Create a mock clock and animation
        let clock = Clock::new();
        let config = LayerOpenAnim::default();
        
        let mut layer = MappedLayer::new(/* ... */);
        layer.start_open_animation(&config);

        // Animation should start at 0.0 progress
        assert!(layer.open_animation.as_ref().unwrap().value() < 0.01);
        
        // Alpha should interpolate from 0.0 to 1.0
        let alpha = layer.animated_alpha();
        assert!(alpha >= 0.0 && alpha <= 1.0);
    }

    #[test]
    fn test_animation_completion() {
        let config = LayerOpenAnim::default();
        let mut layer = MappedLayer::new(/* ... */);
        
        layer.start_open_animation(&config);
        assert!(!layer.are_animations_ongoing() || layer.open_animation.is_some());
        
        // After animation completes
        // layer.open_animation should be None or animation.is_done() should be true
    }

    #[test]
    fn test_disabled_animation() {
        let mut config = LayerOpenAnim::default();
        config.anim.off = true;
        
        let mut layer = MappedLayer::new(/* ... */);
        layer.start_open_animation(&config);
        
        // Animation should not start when disabled
        assert!(layer.open_animation.is_none());
        assert_eq!(layer.animated_alpha(), 1.0); // Fully visible
    }
}
```

### Integration Tests

```rust
#[test]
fn test_layer_open_animation() {
    // Create mock layer surface
    // Map it
    // Verify animation state is created
    // Advance clock
    // Verify animated values change
    // Complete animation
    // Verify layer is fully visible
}

#[test]
fn test_layer_close_animation() {
    // Create and map layer
    // Trigger unmap
    // Verify close animation starts
    // Complete animation
    // Verify layer is hidden
}

#[test]
fn test_animation_interruption() {
    // Start open animation
    // Halfway through, trigger close
    // Verify animation switches to close
}

// DEFERRED: Per-surface animation overrides through layer-rules
// #[test]
// fn test_animation_rule_application() {
//     // Create config with layer rule
//     // Create layer matching rule
//     // Verify rule's animation config is applied
// }
```

### Visual/Manual Testing

1. **Panel animation**: Open waybar, verify smooth fade-in animation
2. **Notification animation**: Trigger dunst notification, verify fade-in effect
3. **Launcher animation**: Open rofi, verify fade animation
4. **Disable animation**: Verify instant appearance when `layer-open { off }` is set
5. **Performance**: Check frame drops with multiple animations

### Test Coverage Goals

| Category | Target |
|----------|--------|
| Unit tests | >90% coverage for new code |
| Integration tests | All major flows |
| Edge cases | Animation interruption, rapid cycles |
| Performance | No regression in render latency |

---

## Implementation Phases

### Phase 1: Foundation (Week 1)

**Goal**: Basic infrastructure and alpha animation (full window parity)

- [x] `LayerOpenAnim` and `LayerCloseAnim` config structs already exist in `niri-config/src/animations.rs`
- [x] `layer_open` and `layer_close` already in `Animations` struct
- [ ] Add animation state fields (`open_animation`, `close_animation`, `open_buffer`, `close_buffer`) to `MappedLayer`
- [ ] Add `start_open_animation()` and `start_close_animation()` methods with snapshot buffer capture
- [ ] Add `animated_alpha()`, `animated_offset()`, `animated_scale()` methods
- [ ] Add `are_animations_ongoing()` method
- [ ] Update `render_normal()` to use animated alpha, offset, and scale
- [ ] Trigger animations in layer shell handler (`layer_shell_handle_commit()`, `layer_destroyed()`)

**Deliverable**: Basic fade animations working for layer surfaces

### Phase 2: Advanced Effects (Weeks 2-3)

**Goal**: Slide, scale, and custom shader support (full window parity)

- [ ] Implement slide offset effect based on layer anchor position
- [ ] Implement scale transform for "popin" effect
- [ ] Add custom shader support (follow window shader pattern)
- [ ] Add integration with frame callback system
- [ ] Handle animation interruption
- [ ] Test animations with different curve types (easing and spring)

**Implementation Notes**:
- Use existing `Animation`, `Kind` (Easing/Spring), `Curve` types from `niri-config`
- Follow same pattern as `WindowOpenAnim`/`WindowCloseAnim` (anim + custom_shader)
- Create shader files: `layer_open_prelude.frag`, `layer_open_epilogue.frag`, etc.

**Deliverable**: Full parity with window open/close animations

### Phase 3: Polish

**Goal**: Refine and test

- [ ] Add unit tests for animation state
- [ ] Add integration tests
- [ ] Performance optimization
- [ ] Documentation (wiki update)
- [ ] Example configurations
- [ ] Bug fixes and edge case handling

**Deliverable**: Production-ready feature

---

## Migration Path

### For Users

No migration required. New config options are optional with sensible defaults.

### For Existing Configs

```kdl
// BEFORE: No layer animations (implicit)
animations {
    // ... window animations only ...
}

// AFTER: Explicit defaults (optional, same behavior)
animations {
    // ... window animations ...

    // NEW: Layer-shell animations (follow same pattern as window animations)
    layer-open {
        duration-ms 200
        curve "ease-out-expo"
    }
    layer-close {
        duration-ms 150
        curve "ease-out-quad"
    }
}
```

### With Custom Shaders

```kdl
animations {
    layer-open {
        duration-ms 300
        curve "ease-out-expo"
        custom-shader "~/.config/niri/shaders/layer-open.glsl"
    }
    layer-close {
        duration-ms 200
        curve "ease-out-quad"
        custom-shader "~/.config/niri/shaders/layer-close.glsl"
    }
}
```

### Disabling Animations

```kdl
animations {
    // Disable all layer animations globally
    layer-open { off }
    layer-close { off }
}
```

### Backward Compatibility

- All new config options follow existing knuffel patterns
- Missing options fall back to sensible defaults (consistent with window animations)
- Existing configs work unchanged
- Animations can be disabled globally via `off` flag

---

## Key Differences from Window Animations

Unlike windows, layer surfaces have unique characteristics:

1. **No Resize Animation**: Layer surfaces are created with fixed dimensions and never resize dynamically. This is a fundamental difference from windows.

2. **Anchor-Based Positioning**: Layer surfaces position themselves relative to screen edges using Wayland anchors (`Anchor::Top`, `Anchor::Bottom`, `Anchor::Left`, `Anchor::Right`). Slide animations should interpolate from the anchored edge.

3. **Namespace Matching**: Layer surfaces are identified by namespace (e.g., "waybar", "dunst", "rofi"). Future layer-rule animation overrides will match by namespace.

4. **No Transaction Protocol**: Unlike windows, layer surfaces don't use the XDG transaction protocol, so close animations don't need to wait for transaction blockers (simplified from window close).

5. **Layer Priority**: Layer surfaces have priority levels (overlay, top, bottom, background). Animation timing may need to account for layer stacking order.

---

## Verification Checklist

Use this checklist to verify implementation completeness:

### Configuration Layer
- [ ] `LayerOpenAnim` struct exists with `anim: Animation` and `custom_shader: Option<String>`
- [ ] `LayerCloseAnim` struct exists with same fields
- [ ] Both are integrated into `Animations` struct
- [ ] Config parsing works for `layer-open` and `layer-close` blocks

### Animation State
- [ ] `MappedLayer` has `open_animation: Option<Animation>` field
- [ ] `MappedLayer` has `close_animation: Option<Animation>` field
- [ ] `MappedLayer` has `open_buffer: Option<TextureBuffer>` for snapshot
- [ ] `MappedLayer` has `close_buffer: Option<TextureBuffer>` for snapshot

### Animation Methods
- [ ] `start_open_animation(&LayerOpenAnim)` creates animation from 0.0 to 1.0
- [ ] `start_close_animation(&LayerCloseAnim)` creates animation from 1.0 to 0.0
- [ ] `animated_alpha()` returns current opacity (0.0-1.0)
- [ ] `animated_offset(anchor)` returns slide offset based on anchor
- [ ] `animated_scale()` returns scale factor (0.5-1.0)
- [ ] `are_animations_ongoing()` checks all active animations
- [ ] `advance_animations()` clears completed animations

### Trigger Integration
- [ ] Layer shell handler triggers open animation on map
- [ ] Layer shell handler triggers close animation on unmap
- [ ] Frame callbacks are sent while animations are active

### Niri.rs Integration
- [ ] `Niri::advance_animations()` calls `mapped_layer.advance_animations()` for each layer
- [ ] Config reload handles `layer_open.custom_shader` changes
- [ ] Config reload handles `layer_close.custom_shader` changes
- [ ] Shader functions `set_custom_layer_open_program()` exist
- [ ] Shader functions `set_custom_layer_close_program()` exist

### Rendering
- [ ] `render_normal()` applies animated alpha to layer surface
- [ ] `render_normal()` applies animated offset for slide effect
- [ ] `render_normal()` applies animated scale for popin effect
- [ ] Custom shaders work with layer animations

### Visual Parity with Windows
- [ ] Fade effect works (0→1 on open, 1→0 on close)
- [ ] Slide effect works (from anchored edge)
- [ ] Scale effect works (0.5→1.0 popin)
- [ ] Custom GLSL shaders work
- [ ] Spring physics animations work
- [ ] All curve types work (easing, spring, deceleration)

---

## Appendix A: Animation Configuration Reference

### Complete Config Schema

```kdl
animations {
    // Window animations (existing)
    window-open { off; }
    window-close { off; }
    window-movement { off; }
    window-resize { off; }
    workspace-switch { off; }

    // NEW: Layer-shell animations (follow same pattern as window animations)
    layer-open {
        duration-ms 200           # Animation duration in ms
        curve "ease-out-expo"     # Easing curve name
        custom-shader "path"      # Optional: path to custom shader
        # OR use spring physics:
        # spring { damping-ratio 0.8; stiffness 1000; epsilon 0.001; }
    }
    layer-close {
        duration-ms 150
        curve "ease-out-quad"
    }
}
```

### Animation Curve Names

Built-in curves (same as window animations):
- `"linear"`
- `"ease-out-quad"` (default for close)
- `"ease-out-cubic"`
- `"ease-out-expo"` (default for open)
- `"cubic-bezier" x1 y1 x2 y2` (custom Bézier curve)

Spring physics (alternative to easing):
```kdl
layer-open {
    spring {
        damping-ratio 0.8       # 0.1 to 10.0
        stiffness 1000          # >= 1
        epsilon 0.001           # 0.00001 to 0.1
    }
}
```

### Custom Shaders

Layer animations support custom GLSL shaders following the same pattern as window animations:

```kdl
animations {
    layer-open {
        duration-ms 300
        curve "ease-out-expo"
        custom-shader "~/.config/niri/shaders/layer-open.glsl"
    }
}
```

Shader interface follows Niri's existing shader conventions (see window animation shaders for reference).

### Deferred: Layer-Rule Animation Overrides

**Note**: Per-surface animation overrides through layer-rules are deferred to a future phase.

When implemented, the pattern would be:
```kdl
layer-rule {
    match namespace="waybar"
    animation-off                    # Disable animations for this surface
}
```

---

## Appendix B: Performance Considerations

### Frame Budget

- Single animated layer: <1ms additional render time
- 10 animated layers: <5ms additional render time
- Animation interpolation: O(1) per animated layer

### Memory Overhead

- Per-layer animation state: ~200 bytes
- Snapshot storage: ~64 bytes per animated layer
- Negligible for typical deployments

### Optimization Strategies

1. **Skip invisible surfaces**: Don't animate if layer is offscreen
2. **Coalesce rapid events**: Combine multiple notifications into one animation
3. **Hardware acceleration**: All transforms happen in GPU via render pipeline
4. **Throttle frame callbacks**: Only request callbacks when animation active

---

## Appendix C: Future Enhancements

### Post-V1 Features

1. **Interactive animations**: Animate layer position in real-time during drag
2. **Physics-based spring animations**: More natural feeling than bezier
3. **Gesture-triggered animations**: Animate layers based on touch gestures
4. **Chained animations**: Sequence multiple animations (slide then fade)
5. **Per-edge animations**: Different directions for different edges of same layer

### Related Features

1. **Layer opacity rules**: Already exists via `opacity` in layer rules
2. **Layer shadows**: Already exists via `shadow` in layer rules
3. **Layer blur control**: Not currently implemented (would need separate implementation)

---

## Appendix D: Glossary

| Term | Definition |
|------|------------|
| Layer-shell | Wayland protocol for overlay surfaces (panels, notifications) |
| MappedLayer | Niri's internal representation of a mapped layer-shell surface |
| Animation state | Data tracking an in-progress animation's progress and values |
| Niri animation model | Unified animation system using `Animation` type with easing/spring curves (not Hyprland-style "styles") |
| Alpha animation | Opacity interpolation from 0.0 to 1.0 (or vice versa) during render |
| Snapshot | Render state captured at animation start for interpolation |
| Curve | Easing function (e.g., ease-out-expo) controlling animation timing |
| Spring | Physics-based animation using damping ratio and stiffness |
| Custom shader | User-provided GLSL shader for advanced animation effects |
| Layer rule | Configuration for specific layer-shell surfaces by namespace |
| Anchor | Wayland layer-shell positioning (Top, Bottom, Left, Right) |
| Transaction | XDG protocol for window state synchronization (not used by layers) |

`````

