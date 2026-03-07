# WINDOW STATE

**Scope:** src/window/ - Window state management

## OVERVIEW

Window state: mapped (visible) vs unmapped (hidden), window properties, decorations.

## STRUCTURE

```
src/window/
├── mod.rs     # Window type definitions
├── mapped.rs  # Mapped window state (1333 lines)
└── unmapped.rs # Unmapped window state
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Window state | `mapped.rs`, `unmapped.rs` |
| Window props | `mod.rs` | Type, title, app_id |

## CONVENTIONS

- Mapped windows have render state
- Unmapped windows pending map

## ANTI-PATTERNS

- State desync between layout and window
- Stale mapped state after client disconnect
