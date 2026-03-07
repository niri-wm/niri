# SCREENCASTING

**Scope:** src/screencasting/ - PipeWire screen capture

## OVERVIEW

Screen capture via PipeWire. xdg-desktop-portal integration, DMA-BUF handling, stream management.

## STRUCTURE

```
src/screencasting/
├── mod.rs     # Screencasting main (797 lines)
└── pw_utils.rs # PipeWire utilities (1573 lines)
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Screen capture | `mod.rs` | Stream setup, DMA-BUF |
| PipeWire ops | `pw_utils.rs` | Unsafe PipeWire bindings |

## CONVENTIONS

- Feature-gated: `#[cfg(feature = "xdp-gnome-screencast")]`
- Unsafe PipeWire FFI

## ANTI-PATTERNS

- Unsafe code in pw_utils.rs (7+ unsafe functions)
- PipeWire version compatibility
- DMA-BUF import failures
