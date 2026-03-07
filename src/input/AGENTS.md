# INPUT KNOWLEDGE BASE

**Scope:** src/input/ - Input handling and event processing

## OVERVIEW

Processes all input events (keyboard, pointer, touch, tablet, gestures) and dispatches to appropriate handlers; implements grab-based interaction model via Smithay.

## STRUCTURE

```
src/input/
├── mod.rs                    # Main dispatcher (5462 lines) - event routing, focus management
├── backend_ext.rs            # Backend abstraction traits (NiriInputBackend/Device)
├── move_grab.rs              # Window move drag operations
├── resize_grab.rs            # Window resize drag operations (pointer)
├── touch_resize_grab.rs      # Window resize drag operations (touch)
├── spatial_movement_grab.rs  # 3D spatial window movement
├── pick_window_grab.rs       # Window selection/picking mode
├── pick_color_grab.rs        # Color picker mode
├── touch_overview_grab.rs    # Touch-based overview gesture
├── swipe_tracker.rs          # Velocity tracking for swipe gestures
├── scroll_tracker.rs         # Scroll event accumulation
└── scroll_swipe_gesture.rs   # Scroll-to-swipe gesture detection
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Event dispatch | `mod.rs:process_input_event()` | Central switch for all InputEvent types |
| Keyboard handling | `mod.rs:on_keyboard()` | Keybinding resolution, modifiers |
| Pointer motion | `mod.rs:on_pointer_motion()` | Cursor positioning, focus changes |
| Touch events | `mod.rs:on_touch_*()` | Down/Motion/Up/Cancel/Frame handling |
| Gesture events | `mod.rs:on_gesture_swipe/pinch/hold_*()` | Touchpad 3+ finger gestures |
| Tablet/stylus | `mod.rs:on_tablet_tool_*()` | Drawing tablet support |
| Implement new grab | `*_grab.rs` files | Implement PointerGrab<State> or TouchGrab<State> |
| Swipe physics | `swipe_tracker.rs` | Velocity calculation for fling gestures |
| Scroll accumulation | `scroll_tracker.rs` | High-res scroll wheel handling |

## CONVENTIONS

- **Grab pattern**: All interactive drags implement `PointerGrab<State>` or `TouchGrab<State>` traits
- **Start data**: Grabs store `PointerGrabStartData` or `TouchGrabStartData` from initiation
- **Unified input**: `PointerOrTouchStartData` enum abstracts pointer vs touch start data
- **Gesture state**: Use `GestureState` enum (Recognizing/Move/ViewOffset) for gesture disambiguation
- **Timestamp handling**: Use `Duration` for monotonic event timestamps
- **Focus clearing**: Grabs call `handle.motion(data, None, event)` to clear client focus during grab
- **Serial tracking**: All input events carry serials for Wayland protocol correctness

## ANTI-PATTERNS

- **FIXME: granular** (80+ occurrences): Marks places needing per-output redraw optimization instead of full redraws
- **FIXME in backend_ext.rs**: Per-device output mapping not fully implemented (lines 21, 29, 36)
- **HACK in mod.rs:460**: Key event filtering with edge case handling
- **Large match blocks**: mod.rs has giant match statements for binds and actions (thousands of lines)
- **FIXME: only redraw window output** in move_grab.rs: Redraw optimization TODOs
