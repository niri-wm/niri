# Architecture Patterns: Layer-Shell Animations

**Domain:** Wayland Compositor - Layer Animation Integration
**Researched:** 2026-02-16

## Recommended Architecture

Layer-shell animations follow the exact same architecture as window animations, using the "snapshot and animate" pattern:

```
Layer Surface Map
       │
       ▼
┌──────────────────┐
│ new_layer_surface│ ──→ Create MappedLayer
└────────┬─────────┘
         │
         ▼ (if animation enabled)
┌──────────────────┐
│ LayerAnimation   │ ──→ Store in MappedLayer
│ ::new_open()     │     (opening_animation field)
└────────┬─────────┘
         │
         ▼
┌──────────────────┐
│ render_normal()  │ ──→ Check for animation
│ checks anim?     │     Call anim.render() if exists
└────────┬─────────┘
         │
         ▼
┌──────────────────┐
│ Animation        │ ──→ Drives progress value
│ advances per     │     over time (spring/easing)
│ frame            │
└────────┬─────────┘
         │
         ▼ (animation complete)
┌──────────────────┐
│ Clean up anim    │ ──→ Remove from MappedLayer
│ Switch to normal │     Render surface normally
└──────────────────┘
```

### Component Boundaries

| Component | Responsibility | Communicates With |
|-----------|---------------|-------------------|
| `LayerAnimation` | Render animation frame, track progress | `MappedLayer` (via stored field) |
| `MappedLayer` | Store anim state, trigger render | `handlers/layer_shell.rs` (triggers), `niri.rs` (advancement) |
| `handlers/layer_shell.rs` | Detect map/unmap events | Calls `MappedLayer` animation methods |
| `niri.rs` | Drive animation clock | Calls `MappedLayer::advance_animations()` |

### Data Flow

1. **Trigger**: Layer surface maps → `handlers/layer_shell.rs` → Create `MappedLayer` with animation
2. **Advance**: Each frame → `niri.rs::advance_animations()` → `MappedLayer` updates animation clock
3. **Render**: Output render → `MappedLayer::render_normal()` → Check animation → Render anim or surface
4. **Complete**: Animation done → Cleanup → Normal rendering

## Patterns to Follow

### Pattern 1: Snapshot-Based Animation

Window animations work by:
1. **Snapshot**: When animation starts, capture current window appearance to offscreen buffer
2. **Animate**: Render from snapshot with transform (scale/alpha) based on animation progress
3. **Complete**: When done, switch to live rendering

Layer animations use identical pattern in `LayerAnimation::render()`:
```rust
// src/layer/layer_animation.rs lines 61-64
let (elem, _sync_point, mut data) = self
    .buffer
    .render(renderer, scale, elements)  // Snapshot
    .context("error rendering layer to offscreen buffer")?;
```

### Pattern 2: Dual-State Storage

Windows store both opening and closing separately:
```rust
// src/layout/tile.rs (conceptual)
pub struct Tile {
    opening_window: Option<OpenAnimation>,
    closing_window: Option<ClosingWindow>,
}
```

Layer should follow:
```rust
// src/layer/mapped.rs (to implement)
pub struct MappedLayer {
    opening_animation: Option<LayerAnimation>,  // NEW
    closing_animation: Option<LayerAnimation>, // NEW
    // ... existing fields
}
```

### Pattern 3: Animation-Aware Rendering

Window tile rendering checks for animation before normal render:
```rust
// src/layout/tile.rs (conceptual)
if let Some(closing) = &self.closing_window {
    return closing.render(...);  // Animate out instead
}
if let Some(opening) = &self.opening_window {
    return opening.render(...);  // Animate in
}
// Normal rendering...
```

Layer should do the same in `render_normal()`.

## Anti-Patterns to Avoid

### Anti-Pattern 1: Triggering Animation After Surface Ready

Window close animation must start **before** `window.on_commit()`:
```rust
// src/handlers/compositor.rs line 269
// Must start the close animation before window.on_commit().
.start_close_animation_for_window(renderer, &window, blocker);
```

Layer close must follow same order—start animation **before** actual unmap.

### Anti-Pattern 2: Blocking on Animation

Animations are non-blocking. The render loop checks `are_animations_ongoing()` and continues rendering even when animating. Don't add blocking waits.

### Anti-Pattern 3: Forgetting Popup Animation

Layer surfaces have popups (dropdowns, menus). Window animations handle this via `render_popups()`. Layer must ensure popups render correctly during animation.

## Scalability Considerations

| Concern | At 100 users | At 10K users | At 1M users |
|---------|--------------|--------------|-------------|
| Animation state | Small per-surface | Manageable | Need efficient cleanup |
| Offscreen buffers | Memory per layer | Keep watch | May need LRU cache |
| Shader compilation | Cached after init | No change | No change |

## Sources

- `src/layer/layer_animation.rs` - Implementation reference
- `src/layout/opening_window.rs` - Snapshot pattern reference
- `src/layout/tile.rs` - State storage pattern
- `src/niri.rs` - Animation advancement loop
