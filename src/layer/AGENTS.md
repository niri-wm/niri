# LAYER-SHELL SURFACES

**Scope:** src/layer/ - Wayland layer-shell surface management

## OVERVIEW

Layer-shell surface handling: panels (waybar, polybar), notifications (dunst), launchers (rofi).

## STRUCTURE

```
src/layer/
├── mod.rs     # Layer surface management
└── mapped.rs  # Mapped layer state
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Layer surfaces | `mod.rs` | Background/overlay panels |
| Layer state | `mapped.rs` | Mapped surface tracking |

## CONVENTIONS

- Layer levels: background, bottom, top, overlay
- Keyboard interactivity modes

## ANTI-PATTERNS

- Layer surfaces not responding to config
- Z-order issues between layers
