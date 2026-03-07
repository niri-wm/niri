# BACKEND DRIVERS

**Scope:** src/backend/ - Graphics backend implementations

## OVERVIEW

Backend drivers for running niri: TTY (real compositor), Winit (debugging), Headless (testing).

## STRUCTURE

```
src/backend/
├── mod.rs      # Backend traits
├── tty.rs      # TTY/console backend (3528 lines)
├── winit.rs    # Winit windowing debug backend
└── headless.rs # Headless testing backend
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Real compositor | `tty.rs` | Direct console/DRM backend |
| Debugging | `winit.rs` | Winit-based window |
| Testing | `headless.rs` | Mock backend |

## CONVENTIONS

- Implement `NiriBackend` trait
- Event-driven via calloop

## ANTI-PATTERNS

- TTY backend requires root/console access
- Winit backend limited Wayland support
