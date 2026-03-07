# Research Summary: Layer-Shell Animations in Niri

**Project:** Niri Wayland Compositor - Layer-Shell Animations
**Researched:** 2026-02-16
**Overall confidence:** HIGH

## Executive Summary

Layer-shell animations for panels (waybar), notifications (dunst), and launchers (rofi) have foundational infrastructure complete but **require integration work** to become functional. The `LayerAnimation` struct exists with full render logic, configuration types are defined, and the shader pipeline supports it—but the code that actually triggers animations on surface map/unmap events is entirely missing.

This follows the classic "building blocks exist, wiring is missing" pattern. The implementation mirrors window animations (`OpeningWindow`/`ClosingWindow`), making remaining work straightforward.

## Key Findings

**Stack:** Uses existing animation system (`Animation`, `Clock`), shader infrastructure (`ProgramType::Open`/`Close`), and offscreen buffer rendering.

**Architecture:** `LayerAnimation` struct in `src/layer/` follows identical pattern to `OpeningWindow` in `src/layout/`. Config stored in `niri-config` alongside window animations.

**Critical gap:** No trigger code calls `LayerAnimation::new_open()` or `new_close()` in the layer-shell handler. Animation state not stored in `MappedLayer`. No render integration.

## Implications for Roadmap

Based on research, suggested implementation approach:

### Phase 1: Animation State and Triggers
- **Add animation fields to `MappedLayer`** - mirror `Tile::opening_window` / `Tile::closing_window`
- **Trigger on surface map** - in `handlers/layer_shell.rs::new_layer_surface()` 
- **Trigger on surface destroy** - in `handlers/layer_shell.rs::layer_destroyed()`
- Addresses: Missing state storage and event triggers
- Avoids: Jumping to complex render integration before basic state works

### Phase 2: Render Integration
- **Call animation render** in `MappedLayer::render_normal()` when animation active
- **Add animation advancement** to render loop
- **Report ongoing status** for render loop optimization
- Addresses: Making animations actually visible

### Phase 3: Completion and Polish
- **Cleanup after animation completes** - actual unmap after close animation
- **Test with real layer surfaces** - waybar, dunst, rofi behavior may differ
- Addresses: Edge cases and real-world testing

**Phase ordering rationale:**
- Phase 1 first because without triggers/state, nothing happens
- Phase 2 before Phase 3 because completion handling depends on render working
- This ordering matches how window animations were likely built

**Research flags for phases:**
- Phase 1: Standard patterns from window code, unlikely to need more research
- Phase 2: May need investigation into render element ordering with popups
- Phase 3: Real usage testing may reveal edge cases (multiple monitors, nested popups)

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | HIGH | Uses existing animation/shader/offscreen systems |
| Features | HIGH | Config (`LayerOpenAnim`/`LayerCloseAnim`) complete |
| Architecture | HIGH | Pattern identical to window animations |
| Pitfalls | MEDIUM | Some uncertainty about popup rendering during animation |

## Gaps to Address

- **Popup rendering during animation**: Window animations may have complex popup handling; layer surfaces also have popups—need to verify same approach works
- **Multiple monitors**: Layer surfaces are per-output; verify animation state handles this correctly
- **Config hot-reloading**: Window animations handle this via `replace_config()`; verify layer animations work the same
- **Testing approach**: No existing tests for layer animations; may need visual tests similar to `niri-visual-tests`

## Files Created

| File | Purpose |
|------|---------|
| `.planning/research/FEATURES.md` | Feature landscape and gap analysis |
| `.planning/research/SUMMARY.md` | This executive summary |
