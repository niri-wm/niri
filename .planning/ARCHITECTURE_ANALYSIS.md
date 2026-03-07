# Layer Animation Architecture Analysis

## Current Implementation (Broken)

### Flow
1. Layer created → inserted into `mapped_layer_surfaces` + Smithay's `layer_map`
2. Layer unmaps → removed from Smithay's `layer_map`, kept in `mapped_layer_surfaces` with `is_closing=true`
3. Rendering → `render_layer_normal` → `layers_in_render_order` → gets layers from Smithay's `layer_map`
4. **BUG:** Unmapped layers aren't in Smithay's layer_map, so they never get rendered!

### Why Window Animations Work

**ClosingWindow pattern:**
```rust
// In ScrollingSpace
closing_windows: Vec<ClosingWindow>

// Rendering - closing windows drawn FIRST
for closing in self.closing_windows.iter().rev() {
    let elem = closing.render(...);
    push(elem.into());
}
// Then live windows are rendered
```

Key insight: **Closing windows are stored separately and rendered explicitly**, not looked up from the window collection.

### Why Hyprland Works

**Hyprland pattern:**
```cpp
// Layer has animated properties
PHLANIMVAR<Vector2D> m_realPosition;
PHLANIMVAR<Vector2D> m_realSize;
PHLANIMVAR<float>    m_alpha;

// On unmap: capture snapshot
makeSnapshot(layer);
startAnimation(layer, OUT);
m_fadingOut = true;

// Visibility check
bool visible() const {
    return (m_mapped && ...) || (m_fadingOut && m_alpha->value() > 0.F);
}

// Rendering
if (pLayer->m_fadingOut) {
    renderSnapshot(pLayer);  // Uses captured snapshot
    return;
}
```

Key insight: **Layer stays in layer list but renders from snapshot when fading out**.

## Required Fix

### Option A: ClosingLayer pattern (like ClosingWindow)

Add to `Niri` struct:
```rust
closing_layers: Vec<(LayerSurface, MappedLayer)>, // or HashMap
```

Modify rendering in `niri.rs`:
```rust
// Render closing layers first (like closing windows)
for (_, mapped) in &self.closing_layers {
    if let Some(elem) = mapped.render_closing(...) {
        push(elem);
    }
}

// Then render live layers from layer_map
push_normal_from_layer!(Layer::Overlay);
// ... etc
```

### Option B: Hyprland-style (keep in layer_map, render snapshot)

Don't remove from Smithay's layer_map immediately. Instead:
1. Mark layer as "fading out"
2. Keep it in layer_map during animation
3. Render from snapshot when fading out

This requires modifying Smithay integration - more complex.

## Recommended: Option A

Simpler, matches existing niri patterns (ClosingWindow), doesn't require Smithay changes.

### Implementation Steps

1. **Add closing_layers storage** to Niri struct
2. **Move layer to closing_layers** when close animation starts
3. **Render closing_layers** explicitly before live layers
4. **Clean up** closing_layers when animation completes
5. **Handle re-open** - if layer re-opens while closing, remove from closing_layers

## Root Causes of Current Bugs

### Bug 1: Wrong Position
- `bob_offset()` applied incorrectly (should be conditional on baba_is_float)
- **Status:** Should be fixed in Plan 05-01

### Bug 2: No Animation (Fuzzel, DMS)
- Animation triggers in `new_layer_surface` ✓
- But layer may not actually be visible/rendered yet
- **Root cause:** Need to ensure layer is properly tracked for rendering

### Bug 3: First Time Only (DMS)
- Animation triggers ✓
- But subsequent opens don't animate because layer was removed from layer_map
- **Root cause:** Same as Bug 2 - layer tracking

### Bug 4: Close Animation Broken
- Close animation starts ✓
- Snapshot may be captured ✓
- But layer not rendered because removed from Smithay's layer_map
- **Root cause:** Closing layers not rendered explicitly

## Summary

The fundamental issue: **We're trying to animate layers that are no longer in the render pipeline.**

Fix: Store closing layers separately (like ClosingWindow) and render them explicitly.
