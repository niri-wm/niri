# CUSTOM WAYLAND PROTOCOLS

**Scope:** src/protocols/ - Custom Wayland protocol implementations

## OVERVIEW

Custom Wayland protocols: virtual pointer/keyboard, screencopy, output management, foreign toplevel.

## STRUCTURE

```
src/protocols/
├── mod.rs                    # Protocol aggregator
├── virtual_pointer.rs        # Virtual pointer (563 lines)
├── virtual_keyboard.rs       # Virtual keyboard
├── screencopy.rs             # Screen capture (742 lines)
├── output_management.rs      # Output config (923 lines)
├── foreign_toplevel.rs       # Foreign toplevel list
├── gamma_control.rs          # Gamma correction
├── ext_workspace.rs          # Workspace protocol (715 lines)
└── raw.rs                    # Raw protocol registration
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Virtual input | `virtual_pointer.rs`, `virtual_keyboard.rs` |
| Screen capture | `screencopy.rs` | xdg-screencast |
| Output config | `output_management.rs` | Monitor arrangement |

## CONVENTIONS

- Register via `wayland_server::protocol::register`
- Implement `Dispatch` trait

## ANTI-PATTERNS

- Protocol version mismatches
- Missing protocol support in clients
