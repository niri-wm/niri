### Overview

<sup>Since: next release</sup>

Niri supports screen zoom (magnification) triggered by a pinch gesture or a key binding.

### Using Zoom

Zoom starts at 1.0× (no zoom) and increases up to the configured `max-zoom`. There are several ways to zoom:

- **Touchpad pinch**: Perform a three-finger pinch gesture on a touchpad to zoom
  in and out.
- **Touchscreen pinch**: Pinch-to-zoom also works on touchscreens.
- **Key binding**: See the [Key Bindings](./Configuration:-Key-Bindings.md#set-zoom-level) page.

Zoom remains active (viewport stays zoomed in) until a gesture or action resets it back to 1.0×.

### Configuration

All zoom settings are configured in the [`zoom {}` config block](./Configuration:-Miscellaneous.md#zoom):
Zoom can also be animated. See the [`animation {}` settings](./Configuration:-Animations.md)

The cursor can optionally scale with zoom via the [`scale-with-zoom`](./Configuration:-Miscellaneous.md#cursor) cursor setting.

### IPC

#### State Query

You can query the current zoom state of all outputs:

```sh
niri msg zoom-state
```

This returns a map from output name to zoom state, where each state contains:

- `level`: the current zoom level (1.0 = no zoom).
- `is_locked`: whether the focal point is locked.
- `focal_x`, `focal_y`: the current focal point in logical pixels.

#### Events

The compositor emits a `ZoomChanged` event whenever the zoom state changes (at
commit granularity, not per animation frame). This event contains the output
name, current zoom level, focal point coordinates, and whether the focal point
is locked.

### Interaction with Other Features

- **Screencasting**: Zoom is taken into account when rendering screen casts. The
  screencast target determines which output's zoom is applied.
- **Screenshots**: The screenshot UI can optionally scale the cursor indicator
  with the zoom level when `scale-with-zoom` is enabled.
- **Lock Screen**: Zoom persists across the lock screen. Key binds with
  `allow-when-locked=true` can still adjust zoom while locked.
