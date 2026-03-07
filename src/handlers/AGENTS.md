# WAYLAND PROTOCOL HANDLERS

**Scope:** src/handlers/ - Wayland protocol request handlers

## OVERVIEW

Wayland protocol handlers: xdg-shell, layer-shell, compositor. Routes Wayland requests to niri actions.

## STRUCTURE

```
src/handlers/
├── mod.rs         # Handler aggregator (842 lines)
├── xdg_shell.rs   # XDG toplevel/popup (1564 lines)
├── layer_shell.rs # Layer-shell (waybar, dunst)
└── compositor.rs # Core compositor protocol
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Window management | `xdg_shell.rs` | Toplevel, popup handling |
| Panels/overlays | `layer_shell.rs` | Waybar, dunst, rofi |
| Compositor | `compositor.rs` | Surface, buffer management |

## CONVENTIONS

- Implement `Handler` trait for each protocol
- Smithay backend dispatch

## ANTI-PATTERNS

- Protocol version mismatches cause silent failures
- Client bugs can crash compositor
