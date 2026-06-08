### Overview

<sup>Since: 25.02</sup>

The `gestures` config section contains gesture settings.
For an overview of all niri gestures, see the [Gestures](./Gestures.md) wiki page.

Here's a quick glance at the available settings along with their default values.

```kdl
gestures {
    dnd-edge-view-scroll {
        trigger-width 30
        delay-ms 100
        max-speed 1500
    }

    dnd-edge-workspace-switch {
        trigger-height 50
        delay-ms 100
        max-speed 1500
    }

    edge-overscroll {
        // resistance 0   // 0 disables this gesture (the default)
    }

    hot-corners {
        // off
        top-left
        // top-right
        // bottom-left
        // bottom-right
    }
}
```

### `dnd-edge-view-scroll`

Scroll the tiling view when moving the mouse cursor against a monitor edge during drag-and-drop (DnD).
Also works on a touchscreen.

This will work for regular drag-and-drop (e.g. dragging a file from a file manager), and for window interactive move when targeting the tiling layout.

The options are:

- `trigger-width`: size of the area near the monitor edge that will trigger the scrolling, in logical pixels.
- `delay-ms`: delay in milliseconds before the scrolling starts.
Avoids unwanted scrolling when dragging things across monitors.
- `max-speed`: maximum scrolling speed in logical pixels per second.
The scrolling speed increases linearly as you move your mouse cursor from `trigger-width` to the very edge of the monitor.

```kdl
gestures {
    // Increase the trigger area and maximum speed.
    dnd-edge-view-scroll {
        trigger-width 100
        max-speed 3000
    }
}
```

### `dnd-edge-workspace-switch`

<sup>Since: 25.05</sup>

Scroll the workspaces up/down when moving the mouse cursor against a monitor edge during drag-and-drop (DnD) while in the overview.
Also works on a touchscreen.

The options are:

- `trigger-height`: size of the area near the monitor edge that will trigger the scrolling, in logical pixels.
- `delay-ms`: delay in milliseconds before the scrolling starts.
Avoids unwanted scrolling when dragging things across monitors.
- `max-speed`: maximum scrolling speed; 1500 corresponds to one screen height per second.
The scrolling speed increases linearly as you move your mouse cursor from `trigger-width` to the very edge of the monitor.

```kdl
gestures {
    // Increase the trigger area and maximum speed.
    dnd-edge-workspace-switch {
        trigger-height 100
        max-speed 3000
    }
}
```

### `edge-overscroll`

<sup>Since: next release</sup>

Push the mouse cursor *past* a true screen edge — a hard edge with no adjacent output, so the motion is clipped — to pan focus to the adjacent column (left/right edge) or workspace (up/down edge).

Unlike `dnd-edge-*`, this works outside drag-and-drop, during normal pointer use. It requires the cursor to reach the *actual* screen boundary (not a proximity band) and keep pushing; the clipped over-travel accumulates and, once it crosses `resistance`, performs a single discrete navigation step. Because the pointer is pinned at a hard edge, incidental edge contact accumulates only a few pixels and cannot trigger it — there is no timer and no heuristic.

It is suppressed while the session is locked, in the overview/screenshot/MRU UIs, during a pointer grab or constraint, and on a fullscreen window (leaving fullscreen must be explicit). It acts on the monitor the cursor is on, fires once per excursion, and re-arms when the cursor returns inside the outputs.

The option is:

- `resistance`: accumulated over-travel past the edge, in logical pixels, required to trigger. `0` (the default) disables the gesture. Higher values require a firmer, more deliberate shove.

This is primarily useful for scrollable layouts run *without* left/right `struts` (combined with `focus-follows-mouse max-scroll-amount="0%"`), where a maximized column leaves no off-screen sliver for ordinary focus-follows-mouse to pan via. Note: with side-by-side monitors of differing height/offset, the L-shaped no-output region next to the shorter monitor is also a true screen edge.

```kdl
gestures {
    // Shove the cursor ~200 px past a screen edge to pan focus
    // to the adjacent column / workspace.
    edge-overscroll {
        resistance 200
    }
}
```

### `hot-corners`

<sup>Since: 25.05</sup>

Put your mouse at the very top-left corner of a monitor to toggle the overview.
Also works during drag-and-dropping something.

`off` disables the hot corners.

```kdl
// Disable the hot corners.
gestures {
    hot-corners {
        off
    }
}
```

<sup>Since: 25.11</sup> You can choose specific hot corners by name: `top-left`, `top-right`, `bottom-left`, `bottom-right`.
If no corners are explicitly set, the top-left corner will be active by default.

```kdl
// Enable the top-right and bottom-right hot corners.
gestures {
    hot-corners {
        top-right
        bottom-right
    }
}
```

You can also customize hot corners per-output [in the output config](./Configuration:-Outputs.md#hot-corners).
