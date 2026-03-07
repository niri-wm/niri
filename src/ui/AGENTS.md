# UI ELEMENTS

**Scope:** src/ui/ - On-screen UI components

## OVERVIEW

UI elements: screenshot, overview, hotkey overlay, exit confirm dialog, MRU window tracking.

## STRUCTURE

```
src/ui/
├── mod.rs                  # UI module
├── screenshot_ui.rs       # Screenshot UI (1205 lines)
├── mru.rs                 # MRU window list (1939 lines)
├── mru/tests.rs           # MRU tests
├── hotkey_overlay.rs      # Keybind display (707 lines)
├── exit_confirm_dialog.rs # Logout dialog
├── screen_transition.rs   # Workspace transitions
└── config_error_notification.rs
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Screenshot | `screenshot_ui.rs` | Screenshot capture UI |
| Window list | `m.rs` | MRU ordering |
| Keybind hints | `hotkey_overlay.rs` | On-screen key display |

## CONVENTIONS

- Render via render_helpers
- Animated transitions

## ANTI-PATTERNS

- UI freezes blocking compositor
- Animation timing issues
