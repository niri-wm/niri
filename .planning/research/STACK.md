# Animation System Stack

**Project:** Niri compositor - layer-shell animations
**Researched:** 2026-02-16
**Confidence:** HIGH

## Overview

Niri's animation system is located in `src/animation/` and provides physics-based (spring) and time-based (easing) animations. The config is in `niri-config/src/animations.rs` with layer-shell animation configs already defined.

## Core Components

### Animation Struct

**Location:** `src/animation/mod.rs:16-29`

```rust
pub struct Animation {
    from: f64,
    to: f64,
    initial_velocity: f64,
    is_off: bool,
    duration: Duration,
    clamped_duration: Duration,
    start_time: Duration,
    clock: Clock,
    kind: Kind,
}
```

| Field | Purpose |
|-------|---------|
| `from` | Starting value |
| `to` | Target value |
| `initial_velocity` | Initial velocity for physics animations |
| `is_off` | Whether animation is disabled |
| `duration` | Total animation duration |
| `clamped_duration` | Time until first reaching target (best effort) |
| `start_time` | When animation started |
| `clock` | Time source (see Clock section) |
| `kind` | Animation type (Easing/Spring/Deceleration) |

### Animation::new()

**Location:** `src/animation/mod.rs:52-71`

```rust
pub fn new(
    clock: Clock,
    from: f64,
    to: f64,
    initial_velocity: f64,
    config: niri_config::Animation,
) -> Self
```

Creates a new animation with default easing, then applies the config to potentially switch to spring or change easing parameters.

**Creation flow:**
1. Creates `EaseOutCubic` easing animation by default
2. Checks if `config.off` is set - if so, returns immediately
3. Calls `replace_config()` to apply user configuration

### Animation::replace_config()

**Location:** `src/animation/mod.rs:73-108`

Allows runtime reconfiguration of an existing animation without restarting it. Preserves `start_time` to maintain animation continuity.

### Animation Kinds

**Location:** `src/animation/mod.rs:31-41`

```rust
enum Kind {
    Easing { curve: Curve },
    Spring(Spring),
    Deceleration { initial_velocity, deceleration_rate },
}
```

#### Easing Curves

**Location:** `src/animation/mod.rs:43-50`

```rust
pub enum Curve {
    Linear,
    EaseOutQuad,
    EaseOutCubic,
    EaseOutExpo,
    CubicBezier(CubicBezier),
}
```

- **Linear:** No easing
- **EaseOutQuad:** Quadratic ease-out
- **EaseOutCubic:** Cubic ease-out (default for many animations)
- **EaseOutExpo:** Exponential ease-out
- **CubicBezier:** Custom bezier curve with 4 parameters (x1, y1, x2, y2)

#### Spring Physics

**Location:** `src/animation/spring.rs`

Based on libadwaita's spring animation (LGPL-2.1-or-later), implementing a damped harmonic oscillator:

```rust
pub struct SpringParams {
    pub damping: f64,      // Calculated from damping_ratio
    pub mass: f64,         // Always 1.0
    pub stiffness: f64,    // From config (u32 converted to f64)
    pub epsilon: f64,      // Convergence threshold
}
```

**Spring configuration (from niri-config):**
```rust
pub struct SpringParams {
    pub damping_ratio: f64,  // 0.1-10.0 (1.0 = critically damped)
    pub stiffness: u32,      // >= 1
    pub epsilon: f64,        // 0.00001-0.1
}
```

### Clock

**Location:** `src/animation/clock.rs`

A shareable, lazy clock that can change rate:

```rust
pub struct Clock {
    inner: Rc<RefCell<AdjustableClock>>,
}
```

**Key methods:**
- `now()` - Returns current animated time (may differ from real time due to rate)
- `set_rate(rate: f64)` - Sets animation speed multiplier (0-1000)
- `set_complete_instantly(bool)` - Makes all animations complete immediately
- `clear()` - Clears cached time for fresh fetch

**Rate usage:** The `rate` is set from `config.animations.slowdown` in `src/niri.rs:1448`:

```rust
let rate = 1.0 / config.animations.slowdown.max(0.001);
clock.set_rate(rate);
```

## Animation Usage Patterns

### Creating an Animation

**From window open (src/layout/tile.rs:548):**
```rust
let anim = Animation::new(
    self.clock.clone(),
    0.,  // from
    1.,  // to
    0.,  // initial_velocity
    config,
);
```

**From workspace switch (src/layout/monitor.rs:473):**
```rust
self.workspace_switch = Some(WorkspaceSwitch::Animation(Animation::new(
    clock,
    from + current,
    0.,
    0.,
    config,
)));
```

### Getting Animation Values

**Basic value:**
```rust
let value = animation.value();  // Returns current interpolated value
```

**Clamped value (stops at target after first reaching):**
```rust
let value = animation.clamped_value();
```

**Checking if done:**
```rust
if animation.is_done() {
    // Animation completed
}
```

## Layer Animation Infrastructure

### Existing LayerAnimation (Not Yet Integrated)

**Location:** `src/layer/layer_animation.rs`

A wrapper for layer-shell surface animations already exists:

```rust
pub struct LayerAnimation {
    anim: Animation,
    random_seed: f32,
    buffer: OffscreenBuffer,
    is_open: bool,
}
```

**Methods:**
- `new_open(anim: Animation)` - Creates open animation
- `new_close(anim: Animation)` - Creates close animation
- `is_done()` - Checks if animation complete
- `clamped_value()` - Gets clamped progress value
- `render(...)` - Renders with animation effects

**Note:** This module exists but is not currently included in the layer module tree (`src/layer/mod.rs` does not export it).

### Layer Animation Config

**Location:** `niri-config/src/animations.rs:206-246`

Already defined and ready to use:

```rust
pub struct LayerOpenAnim {
    pub anim: Animation,
    pub custom_shader: Option<String>,
}

pub struct LayerCloseAnim {
    pub anim: Animation,
    pub custom_shader: Option<String>,
}
```

**Default values:**
- `layer_open`: 200ms, EaseOutExpo
- `layer_close`: 150ms, EaseOutQuad

## Dependencies

### External Crates

| Crate | Version | Purpose |
|-------|---------|---------|
| `keyframe` | 1.1.1 | Easing function traits and implementations |
| `glam` | 0.30.10 | Matrix math for shader transforms |
| `fastrand` | 2.3.0 | Random seed for animation effects |

### Internal Modules

| Module | Purpose |
|--------|---------|
| `src/animation/mod.rs` | Animation struct and timing |
| `src/animation/spring.rs` | Spring physics |
| `src/animation/bezier.rs` | Custom bezier curves |
| `src/animation/clock.rs` | Animation timing clock |

## Implementation Notes for Layer Animations

1. **Clock already available:** `MappedLayer` already has a `clock: Clock` field (`src/layer/mapped.rs:41`)

2. **Config access:** Layer animations use `config.animations.layer_open` and `config.animations.layer_close`

3. **Animation pattern:** Follow the same pattern as `OpenAnimation`/`ClosingWindow` in `src/layout/`

4. **Integration needed:** Add `mod layer_animation` to `src/layer/mod.rs` and integrate into `MappedLayer` state machine

## Sources

- `src/animation/mod.rs` - Animation implementation (HIGH confidence)
- `src/animation/clock.rs` - Clock implementation (HIGH confidence)
- `src/animation/spring.rs` - Spring physics (HIGH confidence)
- `niri-config/src/animations.rs` - Config types (HIGH confidence)
- `src/layer/layer_animation.rs` - Layer animation wrapper (HIGH confidence)
- `src/layer/mapped.rs` - MappedLayer with clock field (HIGH confidence)
