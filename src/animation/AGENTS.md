# ANIMATION SYSTEM

**Scope:** src/animation/ - Animation timing, interpolation, physics

## OVERVIEW

Animation infrastructure: spring physics, bezier curves, clock management, interpolation helpers.

## STRUCTURE

```
src/animation/
├── mod.rs      # Animation types, timing
├── spring.rs   # Spring physics (damping, stiffness)
├── bezier.rs   # Custom bezier curve easing
└── clock.rs    # Animation clock/frame timing
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Spring physics | `spring.rs` | Damped harmonic oscillator |
| Easing curves | `bezier.rs` | Cubic bezier interpolation |
| Animation clock | `clock.rs` | Frame timing |

## CONVENTIONS

- Uses `Duration` for time
- Interpolate via `lerp` methods

## ANTI-PATTERNS

- Animation frame drops cause jitter
- Blocking animation clock stalls compositor
