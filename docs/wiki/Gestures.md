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

<sup>Since: next</sup> Touchpad gestures are now configured as binds in the main `binds {}` block, the same way keyboard shortcuts are. The trigger names are `TouchpadSwipe3Up`, `TouchpadSwipe3Down`, `TouchpadSwipe3Left`, `TouchpadSwipe3Right`, and the equivalent `TouchpadSwipe4*` and `TouchpadSwipe5*` variants for 4- and 5-finger swipes.

The defaults below reproduce the built-in behavior; you can rebind them to any other action or disable them entirely.

```kdl
binds {
    TouchpadSwipe3Up   { focus-workspace-up; }
    TouchpadSwipe3Down { focus-workspace-down; }
    TouchpadSwipe3Left { focus-column-right; }
    TouchpadSwipe3Right { focus-column-left; }
    TouchpadSwipe4Up   { toggle-overview; }
    TouchpadSwipe4Down { toggle-overview; }
}
```

Tuning parameters for touchpad gesture recognition (`recognition-threshold`, `gesture-progress-distance`) live in the `input { touchpad { gestures { } } }` subblock — see [Configuration: Input](./Configuration:-Input.md#touchpad-gesture-tuning).

#### Workspace Switch

Switch workspaces with three-finger vertical swipes (default bind).

#### Horizontal View Movement

Move the view horizontally with three-finger horizontal swipes (default bind).

#### Open and Close the Overview

<sup>Since: 25.05</sup>

Open and close the overview with a four-finger vertical swipe (default bind).

### Touchscreen

<sup>Since: next</sup> Touchscreen gestures are configured as binds in the main `binds {}` block, following the same naming convention as touchpad triggers (`TouchSwipe3Up` parallels `TouchpadSwipe3Up`). There are four families of trigger: multi-finger swipes, multi-finger pinches, multi-finger rotations, and single-finger edge swipes.

#### Swipe Gestures

```kdl
binds {
    TouchSwipe3Up    { focus-workspace-up; }
    TouchSwipe3Down  { focus-workspace-down; }
    TouchSwipe3Left  { focus-column-right; }
    TouchSwipe3Right { focus-column-left; }
    TouchSwipe4Up    { toggle-overview; }
    TouchSwipe4Down  { toggle-overview; }
    // TouchSwipe5Up / TouchSwipe5Down / TouchSwipe5Left / TouchSwipe5Right also available
}
```

Available triggers: `TouchSwipe3Up`, `TouchSwipe3Down`, `TouchSwipe3Left`, `TouchSwipe3Right`, and the equivalent `TouchSwipe4*` and `TouchSwipe5*` variants.

#### Pinch Gestures

```kdl
binds {
    TouchPinch3In  { open-overview; }
    TouchPinch3Out { close-overview; }
    // TouchPinch4In / TouchPinch4Out / TouchPinch5In / TouchPinch5Out also available
}
```

Available triggers: `TouchPinch3In`, `TouchPinch3Out`, `TouchPinch4In`, `TouchPinch4Out`, `TouchPinch5In`, `TouchPinch5Out`. Pinch vs swipe classification is controlled by the `pinch-threshold` and `pinch-ratio` tuning parameters.

#### Rotation Gestures

> [!WARNING]
>
> Rotation detection is an early proof of concept and is currently **buggy and intermittent** on real hardware — recognition can misfire, lock at the wrong finger count, or fail to latch. The math, IPC, and bind plumbing are in place and tests pass, but real-world tuning still needs work. Use with caution and expect false positives / misses while this settles.

Twisting the finger cluster clockwise or counter-clockwise (around its centroid) fires a rotation gesture. Rotation is detected from the averaged per-finger angle change, so two fingers rotating in opposite directions around the centroid both register as the same rotation — the noise floor is √N lower than single-finger angular drift.

```kdl
binds {
    // 4-finger rotation walks column focus left/right.
    TouchRotate4Ccw { focus-column-left; }
    TouchRotate4Cw  { focus-column-right; }
}
```

Available triggers: `TouchRotate3Cw`, `TouchRotate3Ccw`, `TouchRotate4Cw`, `TouchRotate4Ccw`, `TouchRotate5Cw`, `TouchRotate5Ccw`. `Cw` is clockwise on screen, `Ccw` is counter-clockwise on screen — the sign convention assumes the y-axis points down (standard screen coordinates).

Rotation classification runs before pinch and swipe classification, so a clearly rotating finger cluster wins over any incidental spread or translation. Tuning lives under `input { touchscreen { gestures { } } }`: `rotation-threshold` (minimum radians before it latches), `rotation-ratio` (how much rotation arc length must dominate swipe/spread change by), and `rotation-progress-distance` (radians that map to IPC `progress = ±1.0`).

Rotation gestures are **continuous** in the same sense as pinch: binding them to a continuous-capable action animates frame-by-frame, and tagged rotations emit `GestureProgress` events where the delta is `GestureDelta::Rotate { d_radians }`.

Pinch gestures are **continuous**: when bound to a continuous-capable action like `open-overview`, `close-overview`, `toggle-overview`, `focus-workspace-*`, `focus-column-*`, or `noop`, the animation tracks finger motion frame-by-frame (pinch-in smoothly opens the overview, reversing the pinch smoothly closes it again). Binding a pinch to a non-continuous action like `spawn` or `close-window` still fires the action once on recognition, as before.

The animation scale for pinch is controlled by `pinch-sensitivity`, not by the bind's `sensitivity=` property — pinch has its own dedicated knob because raw spread-delta pixels need a very different scaling from linear swipe distances. Tune `pinch-sensitivity` in the `touchscreen { gestures { } }` block if pinch-to-overview feels too fast or too slow.

#### Edge Swipes

One-finger swipes that begin within `edge-threshold` pixels of a screen edge. Useful for drawers, panels, and any edge-activated UI.

```kdl
binds {
    TouchEdgeLeft   { focus-column-right; }
    TouchEdgeRight  { focus-column-left; }
    TouchEdgeTop    { focus-workspace-up; }
    TouchEdgeBottom { focus-workspace-down; }
}
```

Available parent triggers: `TouchEdgeLeft`, `TouchEdgeRight`, `TouchEdgeTop`, `TouchEdgeBottom`. The edge trigger zone width is set by `edge-threshold` in the `touchscreen { gestures { } }` block.

##### Edge zones

<sup>Since: next</sup>

Each edge is also split into three zones along its perpendicular axis so you can bind separate actions to different parts of the same edge (like Android's status bar → notification tray vs. quick-settings split, or a top-right screenshot gesture). Use the zone suffix syntax `TouchEdge<Edge>:<Zone>`. The suffix words rotate per edge to match the direction of the split:

| Edge | Zone suffixes | Meaning |
| --- | --- | --- |
| `TouchEdgeTop` | `:Left` / `:Center` / `:Right` | thirds along the x-axis |
| `TouchEdgeBottom` | `:Left` / `:Center` / `:Right` | thirds along the x-axis |
| `TouchEdgeLeft` | `:Top` / `:Center` / `:Bottom` | thirds along the y-axis |
| `TouchEdgeRight` | `:Top` / `:Center` / `:Bottom` | thirds along the y-axis |

```kdl
binds {
    // Split the top edge into three independent actions.
    TouchEdgeTop:Left    { spawn "notify-send" "left"; }
    TouchEdgeTop:Center  { spawn "notify-send" "pull down notifications"; }
    TouchEdgeTop:Right   { spawn "screenshot.sh"; }

    // Bottom-right corner for the overview; middle-bottom for app drawer.
    TouchEdgeBottom:Center { spawn "rofi" "-show" "drun"; }
    TouchEdgeBottom:Right  { toggle-overview; }

    // Parent bind is still valid. If no zoned bind hits for a given touch,
    // the parent trigger is used as a fallback — so existing configs keep
    // working unchanged, and zone binds simply override the parent on the
    // portion of the edge where they apply.
    TouchEdgeLeft { focus-column-right; }
}
```

The compact `CamelCase` form (e.g. `TouchEdgeTopLeft`) also parses, but the colon-suffixed form is preferred in docs and in the wiki because it makes the parent / zone relationship visible at a glance.

Tuning parameters for touchscreen gesture recognition all live in the `input { touchscreen { gestures { } } }` subblock — see [Configuration: Input](./Configuration:-Input.md#touchscreen).

### Gesture Tags and IPC Events

<sup>Since: next</sup>

Any gesture bind (touchscreen or touchpad) can carry a `tag="..."` property. When the gesture fires, niri emits `GestureBegin`, `GestureProgress`, and `GestureEnd` events on its IPC event stream, carrying the tag string. External applications subscribing to the event stream can react to those events — drive a sidebar drawer, show a scrubbing HUD, move a slider, etc.

```kdl
binds {
    // Tagged workspace switch — still switches workspaces, and also
    // emits GestureProgress events with tag="ws-nav" for external apps
    // that want to show a progress indicator alongside the animation.
    TouchSwipe3Up   tag="ws-nav" { focus-workspace-up; }
    TouchSwipe3Down tag="ws-nav" { focus-workspace-down; }

    // Noop-tagged edge swipe — drives no compositor action, just emits
    // IPC progress events so an external app (e.g. a sidebar drawer)
    // can follow the finger.
    TouchEdgeLeft  tag="sidebar-left"  { noop; }
    TouchEdgeRight tag="sidebar-right" { noop; }
}
```

The three IPC events are:

- **`GestureBegin { tag, trigger, finger_count, is_continuous }`** — fired when gesture recognition has locked in. `is_continuous` is true for swipe, pinch, and edge gestures bound to continuous-capable actions (including `noop`), and false for discrete gestures bound to one-shot actions.
- **`GestureProgress { tag, progress, delta, timestamp_ms }`** — fired repeatedly while a continuous gesture is in motion.
  - `progress` is **signed, unbounded**, normalized: it starts at `0.0` when the gesture is recognized and grows as the gesture continues. Reversing direction produces negative values, and overshoot can exceed `±1.0` — consumers should not assume the value is clamped.
  - For **swipes and edge gestures**, progress accumulates adjusted (sensitivity-scaled, natural-scroll-adjusted) finger delta on the dominant axis, normalized by `gesture-progress-distance` (default 200 px for touchscreen, 40 for touchpad). Progress `±1.0` ≈ one `gesture-progress-distance` of movement.
  - For **pinches**, progress is `(current_spread - start_spread) / pinch-progress-distance` (default 100 px). Positive = pinch-out (spread growing), negative = pinch-in.
  - For **rotations**, progress is cumulative signed rotation in radians divided by `rotation-progress-distance` (default π/2). Positive = counter-clockwise on screen, negative = clockwise on screen.
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
