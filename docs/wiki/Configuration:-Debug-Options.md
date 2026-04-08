### Overview

Niri has several options that are only useful for debugging, or are experimental and have known issues.
They are not meant for normal use.

> [!CAUTION]
> These options are **not** covered by the [config breaking change policy](./Configuration:-Introduction.md#breaking-change-policy).
> They can change or stop working at any point with little notice.

Here are all the options at a glance:

```kdl
debug {
    preview-render "screencast"
    // preview-render "screen-capture"
    enable-overlay-planes
    disable-cursor-plane
    disable-direct-scanout
    restrict-primary-scanout-to-matching-format
    force-disable-connectors-on-resume
    render-drm-device "/dev/dri/renderD129"
    ignore-drm-device "/dev/dri/renderD128"
    ignore-drm-device "/dev/dri/renderD130"
    force-pipewire-invalid-modifier
    dbus-interfaces-in-non-session-instances
    wait-for-frame-completion-before-queueing
    emulate-zero-presentation-time
    disable-resize-throttling
    disable-transactions
    keep-laptop-panel-on-when-lid-is-closed
    disable-monitor-names
    strict-new-window-focus-policy
    honor-xdg-activation-with-invalid-serial
    skip-cursor-only-updates-during-vrr
    deactivate-unfocused-windows
}

binds {
    Mod+Shift+Ctrl+T { toggle-debug-tint; }
    Mod+Shift+Ctrl+O { debug-toggle-opaque-regions; }
    Mod+Shift+Ctrl+D { debug-toggle-damage; }
}
```

### `preview-render`

Make niri render the monitors the same way as for a screencast or a screen capture.

Useful for previewing the `block-out-from` window rule.

```kdl
debug {
    preview-render "screencast"
    // preview-render "screen-capture"
}
```

### `enable-overlay-planes`

Enable direct scanout into overlay planes.
May cause frame drops during some animations on some hardware (which is why it is not the default).

Direct scanout into the primary plane is always enabled.

```kdl
debug {
    enable-overlay-planes
}
```

### `disable-cursor-plane`

Disable the use of the cursor plane.
The cursor will be rendered together with the rest of the frame.

Useful to work around driver bugs on specific hardware.

```kdl
debug {
    disable-cursor-plane
}
```

### `disable-direct-scanout`

Disable direct scanout to both the primary plane and the overlay planes.

```kdl
debug {
    disable-direct-scanout
}
```

### `restrict-primary-scanout-to-matching-format`

Restricts direct scanout to the primary plane to when the window buffer exactly matches the composition swapchain format.

This flag may prevent unexpected bandwidth changes when going between composition and scanout.
The plan is to make it default in the future, when we implement a way to tell the clients the composition swapchain format.
As is, it may prevent some clients (mpv on my machine) from scanning out to the primary plane.

```kdl
debug {
    restrict-primary-scanout-to-matching-format
}
```

### `force-disable-connectors-on-resume`

Force-disables all outputs upon resuming niri (TTY switch or waking up from suspend).
This causes a modeset/screen blank on all outputs.

If niri rendering is corrupted, or monitors don't light up after a TTY switch, you can try this flag.

```kdl
debug {
    force-disable-connectors-on-resume
}
```

### `render-drm-device`

Override the DRM device that niri will use for all rendering.

You can set this to make niri use a different primary GPU than the default one.

```kdl
debug {
    render-drm-device "/dev/dri/renderD129"
}
```

### `ignore-drm-device`

<sup>Since: 25.11</sup>

List DRM devices that niri will ignore.
Useful for GPU passthrough when you don't want niri to open a certain device.

```kdl
debug {
    ignore-drm-device "/dev/dri/renderD128"
    ignore-drm-device "/dev/dri/renderD130"
}
```

### `force-pipewire-invalid-modifier`

<sup>Since: 25.01</sup>

Forces PipeWire screencasting to use the invalid modifier, even when DRM offers more modifiers.

Useful for testing the invalid modifier code path that is hit by drivers that don't support modifiers.

```kdl
debug {
    force-pipewire-invalid-modifier
}
```

### `dbus-interfaces-in-non-session-instances`

Make niri create its D-Bus interfaces even if it's not running as a `--session`.

Useful for testing screencasting changes without having to relogin.

The main niri instance will *not* currently take back the interfaces when you close the test instance, so you will need to relogin in the end to make screencasting work again.

```kdl
debug {
    dbus-interfaces-in-non-session-instances
}
```

### `wait-for-frame-completion-before-queueing`

Wait until every frame is done rendering before handing it over to DRM.

Useful for diagnosing certain synchronization and performance problems.

```kdl
debug {
    wait-for-frame-completion-before-queueing
}
```

### `emulate-zero-presentation-time`

Emulate zero (unknown) presentation time returned from DRM.

This is a thing on NVIDIA proprietary drivers, so this flag can be used to test that niri doesn't break too hard on those systems.

```kdl
debug {
    emulate-zero-presentation-time
}
```

### `disable-resize-throttling`

<sup>Since: 0.1.9</sup>

Disable throttling resize events sent to windows.

By default, when resizing quickly (e.g. interactively), a window will only receive the next size once it has made a commit for the previously requested size.
This is required for resize transactions to work properly, and it also helps certain clients which don't batch incoming resizes from the compositor.

Disabling resize throttling will send resizes to windows as fast as possible, which is potentially very fast (for example, on a 1000 Hz mouse).

```kdl
debug {
    disable-resize-throttling
}
```

### `disable-transactions`

<sup>Since: 0.1.9</sup>

Disable transactions (resize and close).

By default, windows which must resize together, do resize together.
For example, all windows in a column must resize at the same time to maintain the combined column height equal to the screen height, and to maintain the same window width.

Transactions make niri wait until all windows finish resizing before showing them all on screen in one, synchronized frame.
For them to work properly, resize throttling shouldn't be disabled (with the previous debug flag).

```kdl
debug {
    disable-transactions
}
```

### `keep-laptop-panel-on-when-lid-is-closed`

<sup>Since: 0.1.10</sup>

By default, niri will disable the internal laptop monitor when the laptop lid is closed.
This flag turns off this behavior and will leave the internal laptop monitor on.

```kdl
debug {
    keep-laptop-panel-on-when-lid-is-closed
}
```

### `disable-monitor-names`

<sup>Since: 0.1.10</sup>

Disables the make/model/serial monitor names, as if niri fails to read them from the EDID.

Use this flag to work around a crash present in 0.1.9 and 0.1.10 when connecting two monitors with matching make/model/serial.

```kdl
debug {
    disable-monitor-names
}
```

### `strict-new-window-focus-policy`

<sup>Since: 25.01</sup>

Disables heuristic automatic focusing for new windows.
Only windows that activate themselves with a valid xdg-activation token will be focused.

```kdl
debug {
    strict-new-window-focus-policy
}
```

### `honor-xdg-activation-with-invalid-serial`

<sup>Since: 25.05</sup>

Widely-used clients such as Discord and Telegram make fresh xdg-activation tokens upon clicking on their tray icon or on their notification.
Most of the time, these fresh tokens will have invalid serials, because the app needs to be focused to get a valid serial, and if the user clicks on a tray icon or a notification, it is usually because the app *isn't* focused, and the user wants to focus it.

By default, niri ignores xdg-activation tokens with invalid serials, to prevent windows from randomly stealing focus.
This debug flag makes niri honor such tokens, making the aforementioned widely-used apps get focus when clicking on their tray icon or notification.

Amusingly, clicking on a notification sends the app a perfectly valid activation token from the notification daemon, but these apps seem to simply ignore it.
Maybe in the future these apps/toolkits (Electron, Qt) are fixed, making this debug flag unnecessary.

```kdl
debug {
    honor-xdg-activation-with-invalid-serial
}
```

### `skip-cursor-only-updates-during-vrr`

<sup>Since: 25.08</sup>

Skips redrawing the screen from cursor input while variable refresh rate is active.

Useful for games where the cursor isn't drawn internally to prevent erratic VRR shifts in response to cursor movement.

Note that the current implementation has some issues, for example when there's nothing redrawing the screen (like a game), the rendering will appear to completely freeze (since cursor movements won't cause redraws).

```kdl
debug {
    skip-cursor-only-updates-during-vrr
}
```

### `deactivate-unfocused-windows`

<sup>Since: 25.08</sup>

Some clients (notably, Chromium- and Electron-based, like Teams or Slack) erroneously use the Activated xdg window state instead of keyboard focus for things like deciding whether to send notifications for new messages, or for picking where to show an IME popup.
Niri keeps the Activated state on unfocused workspaces and invisible tabbed windows (to reduce unwanted animations), surfacing bugs in these applications.

Set this debug flag to work around these problems.
It will cause niri to drop the Activated state for all unfocused windows.

```kdl
debug {
    deactivate-unfocused-windows
}
```

### `keep-max-bpc-unchanged`

<sup>Since: 25.08</sup>

When connecting monitors, niri sets their max bpc to 8 in order to reduce display bandwidth and to potentially allow more monitors to be connected at once.
Restricting bpc to 8 is not a problem since we don't support HDR or color management yet and can't really make use of higher bpc.

Apparently, setting max bpc to 8 breaks some displays driven by AMDGPU.
If this happens to you, set this debug flag, which will prevent niri from changing max bpc.
AMDGPU bug report: https://gitlab.freedesktop.org/drm/amd/-/issues/4487.

<sup>Since: 25.11</sup>
This setting is deprecated and does nothing: niri no longer sets max bpc.
The old niri behavior with this setting enabled matches the new behavior.

```kdl
debug {
    keep-max-bpc-unchanged
}
```

### Key Bindings

These are not debug options, but rather key bindings.

#### `toggle-debug-tint`

Tints all surfaces green, unless they are being directly scanned out.

Useful to check if direct scanout is working.

```kdl
binds {
    Mod+Shift+Ctrl+T { toggle-debug-tint; }
}
```

#### `debug-toggle-opaque-regions`

<sup>Since: 0.1.6</sup>

Tints regions marked as opaque with blue and the rest of the render elements with red.

Useful to check how Wayland surfaces and internal render elements mark their parts as opaque, which is a rendering performance optimization.

```kdl
binds {
    Mod+Shift+Ctrl+O { debug-toggle-opaque-regions; }
}
```

#### `debug-toggle-damage`

<sup>Since: 0.1.6</sup>

Tints damaged regions with red.

```kdl
binds {
    Mod+Shift+Ctrl+D { debug-toggle-damage; }
}
```
