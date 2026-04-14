### Overview

There are several gestures in niri.

Also see the [gestures configuration](./Configuration:-Gestures.md) wiki page.

### Mouse

#### Interactive Move

<sup>Since: 0.1.10</sup>

You can move windows by holding <kbd>Mod</kbd> and the left mouse button.

You can customize the look of the window insertion preview in the [`insert-hint` layout config](./Configuration:-Layout.md#insert-hint).

<sup>Since: 25.01</sup> Right click while moving to toggle between floating and tiling layout to put the window into.

#### Interactive Resize

<sup>Since: 0.1.6</sup>

You can resize windows by holding <kbd>Mod</kbd> and the right mouse button.

#### Reset Window Height

<sup>Since: 0.1.6</sup>

If you double-click on a top or bottom tiled window resize edge, the window height will reset to automatic.

This works with both window-initiated resizes (when using client-side decorations), and niri-initiated <kbd>Mod</kbd> + right click resizes.

#### Toggle Full Width

<sup>Since: 0.1.6</sup>

If you double-click on a left or right tiled window resize edge, the column will expand to the full workspace width.

This works with both window-initiated resizes (when using client-side decorations), and niri-initiated <kbd>Mod</kbd> + right click resizes.

#### Horizontal View Movement

<sup>Since: 0.1.6</sup>

Move the view horizontally by holding <kbd>Mod</kbd> and the middle mouse button (or the wheel) and dragging the mouse horizontally.

#### Workspace Switch

<sup>Since: 0.1.7</sup>

Switch workspaces by holding <kbd>Mod</kbd> and the middle mouse button (or the wheel) and dragging the mouse vertically.

### Touchpad

<sup>Since: next</sup> Touchpad gestures are configured as binds in the main `binds {}` block, the same way keyboard shortcuts are. The trigger is `TouchpadSwipe` with `fingers=N` (integer in `3..=10`) and `direction="up|down|left|right"` properties.

The defaults below reproduce the built-in behavior; you can rebind them to any other action or disable them entirely.

```kdl
binds {
    TouchpadSwipe fingers=3 direction="up"    { focus-workspace-up; }
    TouchpadSwipe fingers=3 direction="down"  { focus-workspace-down; }
    TouchpadSwipe fingers=3 direction="left"  { focus-column-right; }
    TouchpadSwipe fingers=3 direction="right" { focus-column-left; }
    TouchpadSwipe fingers=4 direction="up"    { toggle-overview; }
    TouchpadSwipe fingers=4 direction="down"  { toggle-overview; }
}
```

Tuning parameters for touchpad gesture recognition (`swipe-trigger-distance`, `swipe-progress-distance`) live in the `input { touchpad { gestures { } } }` subblock — see [Configuration: Input](./Configuration:-Input.md#touchpad-gesture-tuning).

#### Workspace Switch

Switch workspaces with three-finger vertical swipes (default bind).

#### Horizontal View Movement

Move the view horizontally with three-finger horizontal swipes (default bind).

#### Open and Close the Overview

<sup>Since: 25.05</sup>

Open and close the overview with a four-finger vertical swipe (default bind).

#### Tap-Hold Gestures

<sup>Since: next</sup>

Stationary N-finger tap-holds on the touchpad — fingers land, hold stationary, then lift. The action fires on release. libinput handles motion discrimination: if fingers move, the gesture is promoted to a swipe or pinch and the candidate is dropped automatically.

Fast taps (where fingers lift before libinput's internal hold detection threshold) are **not** intercepted — they pass through to the focused client. This means app-level quick-tap gestures (e.g. 3-finger tap-to-paste in terminals) coexist naturally with compositor tap-hold binds.

```kdl
binds {
    TouchpadTapHold fingers=3 { screenshot; }
    TouchpadTapHold fingers=4 { spawn "notify-send" "4-finger tap-hold"; }
    TouchpadTapHold fingers=5 { close-window; }
}
```

- `fingers=` — integer in `3..=10`. Required. 1- and 2-finger holds are handled by libinput and forwarded to clients; niri only intercepts 3+ finger holds.
- No `direction=` — tap-holds are omnidirectional. Including `direction=` is an error.

Tap-holds are always **discrete** (fire-and-forget) — they cannot drive continuous animations.

No niri-side tuning knobs are needed — libinput's hold gesture API handles the motion threshold and timing internally.

#### Tap-Hold-Drag Gestures

<sup>Since: next</sup>

N-finger tap-hold-drag — fingers land, hold stationary, then start moving. The trigger activates when the held fingers begin moving, distinguishing it from a direct swipe (where fingers land already in motion). This is the same gesture macOS uses for three-finger window dragging.

Tap-hold-drag can drive **continuous** actions (workspace switch, overview, view scroll) — the swipe deltas feed into the animation automatically. It can also fire discrete actions once on activation.

```kdl
binds {
    // Continuous: hold 3 fingers, then drag to switch workspaces
    TouchpadTapHoldDrag fingers=3 { focus-workspace-up; }

    // Discrete: hold 4 fingers, then move to trigger once
    TouchpadTapHoldDrag fingers=4 { spawn "notify-send" "drag started"; }
}
```

- `fingers=` — integer in `3..=10`. Required.
- No `direction=` — the drag direction is not part of the trigger. Including `direction=` is an error.

The distinction between tap-hold-drag and a direct swipe is made by libinput: a tap-hold-drag is preceded by a `GestureHoldBegin` event (fingers were stationary first), while a direct swipe skips the hold phase entirely. This means the same finger count can be used for both without conflict — intent is distinguished by the pause before moving.

### Touchscreen

<sup>Since: next</sup> Touchscreen gestures are configured as binds in the main `binds {}` block using six parameterized node families — `TouchSwipe`, `TouchPinch`, `TouchRotate`, `TouchTap`, `TouchTapHoldDrag`, and `TouchEdge` — with KDL properties for finger count and direction. The `fingers=` property accepts any value in `3..=10`, so arbitrary finger counts are supported without an enum change.

#### Swipe Gestures

```kdl
binds {
    TouchSwipe fingers=3 direction="up"    { focus-workspace-up; }
    TouchSwipe fingers=3 direction="down"  { focus-workspace-down; }
    TouchSwipe fingers=3 direction="left"  { focus-column-right; }
    TouchSwipe fingers=3 direction="right" { focus-column-left; }
    TouchSwipe fingers=4 direction="up"    { toggle-overview; }
    TouchSwipe fingers=4 direction="down"  { toggle-overview; }
    // fingers=5 (and 6..=10) also work.
}
```

- `fingers=` — integer in `3..=10`. Rejecting `<3` preserves the 2-finger passthrough contract used by clients for scrolling/zooming. Required.
- `direction=` — one of `"up"`, `"down"`, `"left"`, `"right"`. Required.

#### Pinch Gestures

```kdl
binds {
    TouchPinch fingers=3 direction="in"  { open-overview; }
    TouchPinch fingers=3 direction="out" { close-overview; }
    // fingers=4/5/6/.../10 also work.
}
```

- `fingers=` — integer in `3..=10`. Required.
- `direction=` — one of `"in"` (spread shrinking) or `"out"` (spread growing). Required.

Pinch vs swipe classification is controlled by the `pinch-trigger-distance` and `pinch-dominance-ratio` tuning parameters.

#### Rotation Gestures

> [!WARNING]
>
> Rotation detection is an early proof of concept and is currently **buggy and intermittent** on real hardware — recognition can misfire, lock at the wrong finger count, or fail to latch. The math, IPC, and bind plumbing are in place and tests pass, but real-world tuning still needs work. Use with caution and expect false positives / misses while this settles.

Twisting the finger cluster clockwise or counter-clockwise (around its centroid) fires a rotation gesture. Rotation is detected from the averaged per-finger angle change, so the noise floor is √N lower than single-finger angular drift.

```kdl
binds {
    // 4-finger rotation walks column focus left/right.
    TouchRotate fingers=4 direction="ccw" { focus-column-left; }
    TouchRotate fingers=4 direction="cw"  { focus-column-right; }
}
```

- `fingers=` — integer in `3..=10`. Required.
- `direction=` — one of `"cw"` (clockwise on screen) or `"ccw"` (counter-clockwise on screen). Required. The sign convention assumes the y-axis points down (standard screen coordinates).

Rotation classification runs before pinch and swipe classification, so a clearly rotating finger cluster wins over any incidental spread or translation. Tuning lives under `input { touchscreen { gestures { } } }`: `rotation-trigger-angle` (minimum **degrees** before it latches, default 15°), `rotation-dominance-ratio` (how much rotation arc length must dominate swipe/spread change, default 0.5 — higher = stricter rotation, matching `pinch-dominance-ratio` semantics), and `rotation-progress-angle` (degrees that map to IPC `progress = ±1.0`, default 90°).

Rotation gestures are **continuous** in the same sense as pinch: binding them to a continuous-capable action animates frame-by-frame, and tagged rotations emit `GestureProgress` events where the delta is `GestureDelta::Rotate { d_radians }`.

Pinch gestures are **continuous**: when bound to a continuous-capable action like `open-overview`, `close-overview`, `toggle-overview`, `focus-workspace-*`, `focus-column-*`, or `noop`, the animation tracks finger motion frame-by-frame (pinch-in smoothly opens the overview, reversing the pinch smoothly closes it again). Binding a pinch to a non-continuous action like `spawn` or `close-window` still fires the action once on recognition, as before.

The animation scale for pinch is controlled by `pinch-sensitivity`, not by the bind's `sensitivity=` property — pinch has its own dedicated knob because raw spread-delta pixels need a very different scaling from linear swipe distances. Tune `pinch-sensitivity` in the `touchscreen { gestures { } }` block if pinch-to-overview feels too fast or too slow.

#### Tap Gestures

<sup>Since: next</sup>

Stationary N-finger taps — all fingers land and lift with minimal motion. Tap detection runs in parallel with swipe/pinch/rotate recognition using a spatial dead zone, matching the approach used by Android, iOS, and libinput. If any finger drifts beyond the wobble threshold or the swipe/pinch/rotate recognizer locks first, the tap candidate is killed.

```kdl
binds {
    TouchTap fingers=3 { screenshot; }
    TouchTap fingers=4 { spawn "notify-send" "4-finger tap"; }
    TouchTap fingers=5 { close-window; }
}
```

- `fingers=` — integer in `3..=10`. Required.
- No `direction=` — taps are omnidirectional. Including `direction=` is an error.

Taps are always **discrete** (fire-and-forget) — they cannot drive continuous animations.

Tuning parameters in `input { touchscreen { gestures { } } }`:

- `tap-wobble-threshold` — maximum per-finger displacement (in pixels) before the tap candidate is killed. Default: 15. Increase if taps are too hard to trigger on your device; decrease if taps fire when you intended a swipe.
- `tap-timeout-ms` — maximum duration (in milliseconds) from the third finger landing to all fingers lifting. Default: 250. Acts as a tap-vs-hold safety cap.

The wobble threshold (default 15 px) sits well below the swipe trigger distance (default 100 px), creating a dead zone between 15–100 px where neither tap nor swipe fires — this handles ambiguous gestures correctly.

#### Tap-Hold-Drag Gestures

<sup>Since: next</sup>

N-finger tap-hold-drag — fingers land, hold stationary (within the wobble threshold), then start moving. The trigger fires at the wobble-kill moment — the transition from "was a tap candidate" to "started moving." This distinguishes tap-hold-drag from a direct swipe: direct swipes move immediately without a stationary hold phase.

Tap-hold-drag supports an optional `direction=` property. Directional binds are checked first; if no directional bind matches, the omnidirectional (no `direction=`) bind is used as a fallback.

```kdl
binds {
    // Omnidirectional — fires regardless of initial movement direction
    TouchTapHoldDrag fingers=3 { spawn "notify-send" "drag started"; }

    // Directional — only fires for that initial direction
    TouchTapHoldDrag fingers=4 direction="left"  { spawn "wl-copy"; }
    TouchTapHoldDrag fingers=4 direction="right" { spawn "wl-paste"; }
    TouchTapHoldDrag fingers=4 direction="up"    { toggle-overview; }
}
```

- `fingers=` — integer in `3..=10`. Required.
- `direction=` — optional. One of `"up"`, `"down"`, `"left"`, `"right"`. When omitted, the trigger is omnidirectional.

Tap-hold-drag can drive **continuous** actions — when bound to a continuous-capable action, the swipe deltas feed into the animation frame-by-frame after activation. Binding to a discrete action fires it once.

Tuning parameters in `input { touchscreen { gestures { } } }`:

- `tap-hold-trigger-delay-ms` — minimum hold duration (in milliseconds) before a wobble-kill can activate a tap-hold-drag bind. If fingers move before this delay elapses, normal swipe/pinch/rotate recognition continues instead. Default: 150. Increase if fast swipes accidentally trigger hold-drag; decrease if hold-drag feels sluggish to activate.

The hold detection also reuses the tap candidate's wobble threshold (`tap-wobble-threshold`, default 15 px). Fingers must stay within this threshold during the hold phase.

#### Edge Swipes

One-finger swipes that begin within `edge-start-distance` pixels of a screen edge. Useful for drawers, panels, and any edge-activated UI.

```kdl
binds {
    TouchEdge edge="left"   { focus-column-right; }
    TouchEdge edge="right"  { focus-column-left; }
    TouchEdge edge="top"    { focus-workspace-up; }
    TouchEdge edge="bottom" { focus-workspace-down; }
}
```

- `edge=` — one of `"left"`, `"right"`, `"top"`, `"bottom"`. Required.
- `zone=` — optional third-of-the-edge qualifier (see Edge Zones below).
- No `fingers=` — edge swipes are always single-finger. Including `fingers=` is an error.

The edge trigger zone width is set by `edge-start-distance` in the `touchscreen { gestures { } }` block.

##### Edge swipes with continuous actions (overview, workspace switch)

Edge swipes can be bound to continuous actions like `toggle-overview` or `focus-workspace-up`. Two things to be aware of:

- **Direction inversion:** Edge swipes feeding into overview require `natural-scroll=true` to feel correct. Without it, swiping down from the top edge tries to close overview instead of opening it.

  ```kdl
  binds {
      TouchEdge edge="top" zone="right" natural-scroll=true { toggle-overview; }
  }
  ```

- **Left/right edges and overview:** Continuous overview gestures currently only track vertical (`delta_y`) motion. Left and right edge swipes produce primarily horizontal motion (`delta_x`), which the overview ignores. This means `toggle-overview` on a left or right edge swipe will not work. Use top or bottom edges for overview binds. This is a known limitation.

##### Edge zones

<sup>Since: next</sup>

Each edge is also split into three zones along its perpendicular axis so you can bind separate actions to different parts of the same edge (like Android's status bar → notification tray vs. quick-settings split, or a top-right screenshot gesture). Add a `zone=` property to restrict the bind to one third. The zone vocabulary rotates per edge to match the direction of the split:

| Edge | Valid `zone=` values | Meaning |
| --- | --- | --- |
| `edge="top"` | `"left"` / `"center"` / `"right"` | thirds along the x-axis |
| `edge="bottom"` | `"left"` / `"center"` / `"right"` | thirds along the x-axis |
| `edge="left"` | `"top"` / `"center"` / `"bottom"` | thirds along the y-axis |
| `edge="right"` | `"top"` / `"center"` / `"bottom"` | thirds along the y-axis |

Mismatched vocabularies (e.g. `edge="left" zone="left"`) are a parse error.

```kdl
binds {
    // Split the top edge into three independent actions.
    TouchEdge edge="top" zone="left"    { spawn "notify-send" "left"; }
    TouchEdge edge="top" zone="center"  { spawn "notify-send" "pull down notifications"; }
    TouchEdge edge="top" zone="right"   { spawn "screenshot.sh"; }

    // Bottom-right corner for the overview; middle-bottom for app drawer.
    TouchEdge edge="bottom" zone="center" { spawn "rofi" "-show" "drun"; }
    TouchEdge edge="bottom" zone="right"  { toggle-overview; }

    // Parent bind is still valid. If no zoned bind hits for a given touch,
    // the parent (no `zone=`) trigger is used as a fallback — so a bare
    // `TouchEdge edge="left"` catches any left-edge swipe that doesn't land
    // in a more specific zone bind.
    TouchEdge edge="left" { focus-column-right; }
}
```

Tuning parameters for touchscreen gesture recognition all live in the `input { touchscreen { gestures { } } }` subblock — see [Configuration: Input](./Configuration:-Input.md#touchscreen).

### Gesture Tags and IPC Events

<sup>Since: next</sup>

Any gesture bind (touchscreen or touchpad) can carry a `tag="..."` property. When the gesture fires, niri emits `GestureBegin`, `GestureProgress`, and `GestureEnd` events on its IPC event stream, carrying the tag string. External applications subscribing to the event stream can react to those events — drive a sidebar drawer, show a scrubbing HUD, move a slider, etc.

```kdl
binds {
    // Tagged workspace switch — still switches workspaces, and also
    // emits GestureProgress events with tag="ws-nav" for external apps
    // that want to show a progress indicator alongside the animation.
    TouchSwipe fingers=3 direction="up"   tag="ws-nav" { focus-workspace-up; }
    TouchSwipe fingers=3 direction="down" tag="ws-nav" { focus-workspace-down; }

    // Noop-tagged edge swipe — drives no compositor action, just emits
    // IPC progress events so an external app (e.g. a sidebar drawer)
    // can follow the finger.
    TouchEdge edge="left"  tag="sidebar-left"  { noop; }
    TouchEdge edge="right" tag="sidebar-right" { noop; }
}
```

The three IPC events are:

- **`GestureBegin { tag, trigger, finger_count, is_continuous }`** — fired when gesture recognition has locked in. `is_continuous` is true for swipe, pinch, and edge gestures bound to continuous-capable actions (including `noop`), and false for discrete gestures bound to one-shot actions.
- **`GestureProgress { tag, progress, delta, timestamp_ms }`** — fired repeatedly while a continuous gesture is in motion.
  - `progress` is **signed, unbounded**, normalized: it starts at `0.0` when the gesture is recognized and grows as the gesture continues. Reversing direction produces negative values, and overshoot can exceed `±1.0` — consumers should not assume the value is clamped.
  - For **swipes and edge gestures**, progress accumulates adjusted (sensitivity-scaled, natural-scroll-adjusted) finger delta on the dominant axis, normalized by `swipe-progress-distance` (default 200 px for touchscreen, 40 libinput units for touchpad — same knob name, separate config block). Progress `±1.0` ≈ one progress-distance of movement.
  - For **pinches**, progress is `(current_spread - start_spread) / pinch-progress-distance` (default 100 px). Positive = pinch-out (spread growing), negative = pinch-in.
  - For **rotations**, progress is cumulative signed rotation divided by `rotation-progress-angle` (configured in **degrees**, default 90°). Positive = counter-clockwise on screen, negative = clockwise on screen.
  - `delta` is a tagged enum carrying the per-event raw delta in a gesture-specific shape:
    - `GestureDelta::Swipe { dx, dy }` — per-event finger delta in screen pixels (touchscreen) or libinput units (touchpad).
    - `GestureDelta::Pinch { d_spread }` — per-event change in finger spread.
    - `GestureDelta::Rotate { d_radians }` — per-event change in the averaged per-finger angle. Signed with the same on-screen convention as `progress`.
- **`GestureEnd { tag, completed }`** — fired when the gesture ends (fingers released).

#### Noop Gestures

Binding a tagged gesture to `noop` means the gesture emits IPC events without driving any compositor animation. This is the cleanest case for external apps: progress is the sole output, and the external app has full control over its own thresholds and snap behavior. Used by [niri-tag-sidebar](https://github.com/julianjc84/niri-tag-sidebar) for edge-swipe drawer panels.

#### Progress vs Compositor Animation

> [!WARNING]
>
> When a tagged gesture *also* drives a compositor animation (e.g. a tagged workspace switch), niri uses its own internal thresholds to decide when to commit the action — these are independent of the IPC `progress` value. An external app watching the progress value can't reliably predict when niri will actually commit. For `noop` gestures this isn't a concern because progress is the sole output.

The `GestureEnd.completed` field is currently hardcoded `true` for touchscreen gestures and does **not** indicate whether niri actually committed the bound action.

### All Pointing Devices

#### Drag-and-Drop Edge View Scroll

<sup>Since: 25.02</sup>

Scroll the tiling view when moving the mouse cursor against a monitor edge during drag-and-drop (DnD).
Also works on a touchscreen.

#### Drag-and-Drop Edge Workspace Switch

<sup>Since: 25.05</sup>

Scroll the workspaces up/down when moving the mouse cursor against a monitor edge during drag-and-drop (DnD) while in the overview.
Also works on a touchscreen.

#### Drag-and-Drop Hold to Activate

<sup>Since: 25.05</sup>

While drag-and-dropping, hold your mouse over a window to activate it.
This will bring a floating window to the top for example.

In the overview, you can also hold the mouse over a workspace to switch to it.

#### Hot Corner to Toggle the Overview

<sup>Since: 25.05</sup>

Put your mouse at the very top-left corner of a monitor to toggle the overview.
Also works during drag-and-dropping something.
