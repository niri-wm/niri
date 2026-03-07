# LAYOUT MODULE

Window layout engine for scrollable tiling with dynamic workspaces.

## OVERVIEW

Implements niri's core scrollable-tiling paradigm: windows arranged in columns on an infinite horizontal strip, with workspaces arranged vertically. Supports both tiling and floating window modes.

## STRUCTURE

```
./
├── mod.rs              # Layout trait, window management, interactive move
├── scrolling.rs        # Scrollable-tiling space (5600+ lines - core algorithm)
├── tile.rs            # Individual window wrapper with decorations
├── workspace.rs       # Workspace combining scrolling + floating spaces
├── floating.rs        # Floating window space
├── monitor.rs         # Output managing multiple workspaces
├── tests.rs           # Property-based randomized tests (3900+ lines)
├── tests/             # Additional test modules
│   ├── animations.rs
│   └── fullscreen.rs
├── focus_ring.rs      # Focus indicator rendering
├── shadow.rs          # Window shadows
├── opening_window.rs  # Window open animations
├── closing_window.rs  # Window close animations
└── tab_indicator.rs   # Tab group indicators
```

## WHERE TO LOOK

| Task | File | Notes |
|------|------|-------|
| Scrollable tiling algorithm | `scrolling.rs` | Column management, view offset, gestures |
| Window decorations | `tile.rs` | Borders, focus ring, shadows, animations |
| Workspace logic | `workspace.rs` | Original output tracking, scrolling+floating switching |
| Floating windows | `floating.rs` | Free-floating window positioning |
| Multi-monitor | `monitor.rs` | Workspace switching, overview zoom |
| Window open/close | `opening_window.rs`, `closing_window.rs` | Animation states |
| Layout tests | `tests.rs` | Property-based randomized testing |

## CONVENTIONS

- **Scrollable-tiling**: Windows in columns, scroll horizontally with `ColumnWidth` and `ViewOffset`
- **Dynamic workspaces**: Empty workspace always at bottom; workspaces move between monitors preserving original output
- **LayoutElement trait**: Abstract window interface in `mod.rs` for testability
- **Space pattern**: Both `ScrollingSpace` and `FloatingSpace` manage `Tile<W>` collections
- **Render elements**: Each component has corresponding `*RenderElement` for Smithay rendering

## ANTI-PATTERNS

- **Complex view offset logic**: `scrolling.rs` has subtle view positioning; test gesture edge cases
- **Interactive move state**: Three-phase state (Starting/Moving) in `mod.rs` with complex coordinate transforms
- **Floating position caching**: `SizeFrac` vs logical coords requires careful conversion in `floating.rs`
- **Fullscreen/maximize transitions**: Size mode changes are async; states tracked in both `Tile` and window
- **FIXME in monitor.rs**: Workspace switch animation cleanup when removing workspaces
</content>