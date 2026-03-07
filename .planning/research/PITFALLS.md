# Domain Pitfalls: Layer-Shell Animations

**Domain:** Wayland Compositor - Layer Animation Implementation
**Researched:** 2026-02-16

## Critical Pitfalls

### Pitfall 1: Triggering Close Animation After Unmap

**What goes wrong:** Layer surface disappears instantly without animation, or crash due to accessing unmapped surface.

**Why it happens:** Unlike windows, layer surfaces use `layer_destroyed()` callback which is the actual removal. Must start animation **before** removing from `mapped_layer_surfaces`.

**Consequences:** 
- No animation visible (surface removed immediately)
- Potential panic if animation tries to render already-destroyed surface
- Inconsistent behavior compared to windows

**Prevention:** Follow window pattern from `compositor.rs`:
```rust
// WRONG - triggers after surface effectively gone
fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
    // Start animation here - TOO LATE
}

// CORRECT - start animation before destruction
// (similar to how xdg_shell.rs handles window close)
```

**Detection:** Add assertion that animation starts before any removal.

---

### Pitfall 2: Forgetting Popups in Animation Render

**What goes wrong:** Layer surface popups (dropdowns, menus) don't animate or render incorrectly.

**Why it happens:** Layer surfaces have separate popup handling (`render_popups()`). If animation only covers normal rendering, popups render live during snapshot-based animation.

**Consequences:** Visual artifacts during animation - popups appear full-res while surface animates

**Prevention:** Ensure both `render_normal()` and `render_popups()` check for active animation and handle appropriately.

---

### Pitfall 3: Animation State Not Cleaned Up

**What goes wrong:** Animation object persists after completion, wasting memory and causing incorrect rendering.

**Why it happens:** Forgetting to clear `opening_animation` / `closing_animation` after `is_done()` returns true.

**Consequences:**
- Memory leak (small but accumulates)
- Wrong render path (always animating instead of normal)
- Render loop never optimizes (always thinks animation ongoing)

**Prevention:** In animation advancement, check `is_done()` and clear state:
```rust
pub fn advance_animations(&mut self) {
    if let Some(ref mut anim) = self.opening_animation {
        if anim.is_done() {
            self.opening_animation = None;  // Clean up!
        }
    }
}
```

---

## Moderate Pitfalls

### Pitfall 4: Wrong Clock Used for Animation

**What goes wrong:** Animation runs too fast/slow or doesn't sync with other animations.

**Why it happens:** Using wrong clock (e.g., creating new `Clock` instead of using `MappedLayer`'s existing clock).

**Consequences:** Inconsistent animation timing, potential desync

**Prevention:** Pass `MappedLayer`'s existing clock to animation constructor (same as window code does).

---

### Pitfall 5: Multiple Monitors with Same Namespace

**What goes wrong:** Animation triggers on wrong output or duplicates.

**Why it happens:** Layer surfaces can appear on multiple outputs. Animation state must be per-output, per-surface.

**Consequences:** Wrong animation on wrong monitor, or animation triggering multiple times

**Prevention:** Each `MappedLayer` is already per-surface, per-output. Ensure animation state follows same lifecycle.

---

### Pitfall 6: Custom Shader Not Loading

**What goes wrong:** User configures custom shader but it doesn't apply.

**Why it happens:** Shader loading can fail silently (see shader anti-patterns in AGENTS.md). Code must handle `program() returns None`.

**Consequences:** Animation falls back to simple fade/scale instead of custom effect

**Prevention:** Follow pattern in `layer_animation.rs` - check shader availability:
```rust
if Shaders::get(renderer).program(program_type).is_some() {
    // Use custom shader
} else {
    // Fall back to simple animation
}
```

---

## Minor Pitfalls

### Pitfall 7: Config Hot-Reload Not Working

**What goes wrong:** Changing animation config doesn't update in-progress animations.

**Why it happens:** Not calling `animation.replace_config()` when config changes.

**Prevention:** Niri reloads config at runtime; ensure `MappedLayer` watches for animation config changes and updates in-progress animations.

---

### Pitfall 8: Blocking Animation Blocks Compositor

**What goes wrong:** Long-running animation (e.g., very slow spring) blocks main thread.

**Why it happens:** Animation calculations in same thread as compositor event loop.

**Consequences:** Input lag, visual jank

**Prevention:** Keep animation calculations fast (they are - this is mostly a concern if adding complex physics).

---

## Phase-Specific Warnings

| Phase Topic | Likely Pitfall | Mitigation |
|-------------|---------------|------------|
| Phase 1: Triggers | Trigger timing wrong (after vs before unmap) | Follow window close pattern explicitly |
| Phase 1: State | Forgetting to store animation in MappedLayer | Mirror Tile struct fields |
| Phase 2: Render | Popups not animating with surface | Check both render paths |
| Phase 2: Advancement | Memory leak from uncleaned animation | Clear after is_done() |
| Phase 3: Testing | Multiple monitors behave differently | Test with waybar on 2+ outputs |

---

## Sources

- `src/handlers/compositor.rs` - Window close trigger pattern (line 269)
- `src/layout/tile.rs` - Animation state cleanup pattern
- `src/render_helpers/AGENTS.md` - Shader failure handling
- `src/animation/mod.rs` - Clock usage patterns
