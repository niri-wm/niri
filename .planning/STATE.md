# State: Layer-Shell Animations

**Last Updated:** 2026-02-17

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-16)

**Core Value:** Layer-shell surfaces animate smoothly on open/close with configurable fade, slide, and scale effects.

**Current Focus:** Phase 5: Complete

## Current Position

Phase: 5 of 5 (Edge Cases & Testing)
Plan: 3 of 3 in current phase
Status: Phase complete
Last activity: 2026-02-17 - Completed quick task 005: Fix Fuzzel hang and delayed layer open animation (commit aed128eb)

Progress: █████████████ 100%

## Current Status

| Phase | Status | Progress |
|-------|--------|----------|
| 1: Animation Infrastructure | Complete | 100% |
| 2: Animation Triggers | Complete | 100% |
| 3: Rendering Integration | Complete | 100% |
| 4: Custom Shaders | Complete | 100% |
| 5: Edge Cases & Testing | Complete | 100% |

## Recent Changes

- 2026-02-16: Project initialized with research and roadmap
- 2026-02-16: Completed Phase 1 Plan 1 - Animation infrastructure added to MappedLayer
- 2026-02-16: Completed Phase 2 Plan 1 - Animation triggers wired for map/unmap events
- 2026-02-16: Completed Phase 3 Plan 1 - Layer animations integrated into rendering pipeline
- 2026-02-16: Completed Phase 4 Plan 1 - Custom shader wiring for layer animations
- 2026-02-17: Completed Phase 5 Plan 1 - Fixed wrong position bug during layer animations
- 2026-02-17: Completed Phase 5 Plan 2 - Fixed open animation for persistent launchers (Fuzzel, DMS)
- 2026-02-17: Completed Phase 5 Plan 3 - Fixed close animation to trigger in unmap path with snapshot capture
- 2026-02-17: Completed Phase 5 Plan 4 - Implemented ClosingLayer pattern for close animations

## Notes

- LayerAnimation struct exists at `src/layer/layer_animation.rs` - needs integration ✓ (integrated)
- Config already implemented: `layer_open` and `layer_close` in animations.rs ✓ (ready to use)
- MappedLayer already has `clock: Clock` field ✓ (used for animations)
- Animation triggers implemented ✓ (open on map, close on unmap/destroy)
- Animation rendering integrated ✓ (render_with_animation in mapped.rs, render_layer_normal updated)
- Custom shader wiring complete ✓ (TTY/Winit startup + hot-reload in niri.rs)
- Close animation snapshot capture implemented ✓

## Decisions Made

| Phase | Decision | Rationale |
|-------|----------|-----------|
| 01-01 | Follow Tile pattern for animation state | Established pattern in layout module, maintains consistency |
| 02-01 | Approach B for close animation: keep layer alive until animation completes | Ensures animation can advance each frame, cleanup after done |
| 03-01 | Blocked out layers fall back to normal rendering | Simplifies implementation, avoids complex blocked-out animation handling |
| 04-01 | Reuse existing set_custom_open/close_program functions | Shares shader infrastructure with windows, simpler implementation |
| 05-03 | Use RefCell for interior mutability in LayerAnimation snapshot | Allows snapshot capture without changing method signatures |
| 05-04 | Use stored geometry instead of layer_map lookup | layer_map doesn't contain unmapped layers |

## Blockers/Concerns Carried Forward

None

## Quick Tasks Completed

| # | Description | Date | Commit | Directory |
|---|-------------|------|--------|-----------|
| 001 | Fix layer animation bugs (position, persistent launcher, close animation, DMS spotlight) | 2026-02-17 | 0a5458ae | [001-fix-layer-animations](./quick/001-fix-layer-animations/) |
| 002 | Capture snapshot immediately (partial - added empty check workaround) | 2026-02-17 | 09e36a23 | [002-capture-snapshot-immediately](./quick/002-capture-snapshot-immediately/) |
| 003 | Refactor snapshot capture to preserve close animation content | 2026-02-17 | b820586a | [003-refactor-snapshot-capture](./quick/003-refactor-snapshot-capture/) |
| 004 | Fix layer animation lifecycle bugs (closing_layers redraw, destroy geometry) | 2026-02-17 | ac02301a | [004-fix-layer-animation-lifecycle-bugs-from](./quick/004-fix-layer-animation-lifecycle-bugs-from/) |
| 005 | Fix Fuzzel hang and delayed layer open animation (remove stale layer from Smithay map) | 2026-02-17 | aed128eb | [005-fix-fuzzel-hang-and-delayed-layer-open-a](./quick/005-fix-fuzzel-hang-and-delayed-layer-open-a/) |

---

*State updated: 2026-02-17*
