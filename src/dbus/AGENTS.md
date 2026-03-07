# D-BUS INTEGRATION

**Scope:** src/dbus/ - D-Bus service integration

## OVERVIEW

D-Bus integration for systemd, GNOME session, screen reader support.

## STRUCTURE

```
src/dbus/
└── mod.rs  # D-Bus services
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| D-Bus services | `mod.rs` | systemd, a11y, screen cast |

## CONVENTIONS

- Feature-gated: `#[cfg(feature = "dbus")]`
- Uses `zbus` crate

## ANTI-PATTERNS

- D-Bus unavailable on some systems
