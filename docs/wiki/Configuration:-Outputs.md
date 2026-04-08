### Overview

By default, niri will attempt to turn on all connected monitors using their preferred modes.
You can disable or adjust this with `output` sections.

Here's what it looks like with all properties written out:

```kdl
output "eDP-1" {
    // off
    mode "1920x1080@120.030"
    scale 2.0
    transform "90"
    position x=1280 y=0
    variable-refresh-rate // on-demand=true
    focus-at-startup
    backdrop-color "#001100"

    hot-corners {
        // off
        top-left
        // top-right
        // bottom-left
        // bottom-right
    }

    layout {
        // ...layout settings for eDP-1...
    }

    // Custom modes. Caution: may damage your display.
    // mode custom=true "1920x1080@100"
    // modeline 173.00  1920 2048 2248 2576  1080 1083 1088 1120 "-hsync" "+vsync"
}

output "HDMI-A-1" {
    // ...settings for HDMI-A-1...
}

output "Some Company CoolMonitor 1234" {
    // ...settings for CoolMonitor...
}
```

Outputs are matched by connector name (i.e. `eDP-1`, `HDMI-A-1`), or by monitor manufacturer, model, and serial, separated by a single space each.
You can find all of these by running `niri msg outputs`.

Usually, the built-in monitor in laptops will be called `eDP-1`.

<sup>Since: 0.1.6</sup> The output name is case-insensitive.

<sup>Since: 0.1.9</sup> Outputs can be matched by manufacturer, model, and serial.
Before, they could be matched only by the connector name.

### `off`

This flag turns off that output entirely.

```kdl
// Turn off that monitor.
output "HDMI-A-1" {
    off
}
```

### `mode`

Set the monitor resolution and refresh rate.

The format is `<width>x<height>` or `<width>x<height>@<refresh rate>`.
If the refresh rate is omitted, niri will pick the highest refresh rate for the resolution.

If the mode is omitted altogether or doesn't work, niri will try to pick one automatically.

Run `niri msg outputs` while inside a niri instance to list all outputs and their modes.
The refresh rate that you set here must match *exactly*, down to the three decimal digits, to what you see in `niri msg outputs`.

```kdl
// Set a high refresh rate for this monitor.
// High refresh rate monitors tend to use 60 Hz as their preferred mode,
// requiring a manual mode setting.
output "HDMI-A-1" {
    mode "2560x1440@143.912"
}

// Use a lower resolution on the built-in laptop monitor
// (for example, for testing purposes).
output "eDP-1" {
    mode "1280x720"
}
```

#### `mode custom=true`

<sup>Since: 25.11</sup>

You can configure a custom mode (not offered by the monitor) by setting `custom=true`.
In this case, the refresh rate is mandatory.

Custom modes are not guaranteed to work.
Niri is asking the monitor to run in a mode that is not supported by the manufacturer.
Use at your own risk.

> [!CAUTION]
> Custom modes may damage your monitor, especially if it's a CRT.
> Follow the maximum supported limits in your monitor's instructions.

```kdl
// Use a custom mode for this display.
output "HDMI-A-1" {
    mode custom=true "2560x1440@143.912"
}
```

### `modeline`

<sup>Since: 25.11</sup>

Directly configures the monitor's mode via a modeline, overriding any configured `mode`.
The modeline can be calculated via utilities such as [cvt](https://man.archlinux.org/man/cvt.1.en) or [gtf](https://man.archlinux.org/man/gtf.1.en).

Modelines are not guaranteed to work.
Niri is asking the monitor to run in a mode not supported by the manufacturer.
Use at your own risk.

> [!CAUTION]
> Out of spec modelines may damage your monitor, especially if it's a CRT.
> Follow the maximum supported limits in your monitor's instructions.

```kdl
// Use a modeline for this display.
output "eDP-3" {
    modeline 173.00  1920 2048 2248 2576  1080 1083 1088 1120 "-hsync" "+vsync"
}
```

### `scale`

Set the scale of the monitor.

<sup>Since: 0.1.6</sup> If scale is unset, niri will guess an appropriate scale based on the physical dimensions and the resolution of the monitor.

<sup>Since: 0.1.7</sup> You can use fractional scale values, for example `scale 1.5` for 150% scale.

<sup>Since: 0.1.7</sup> Dot is no longer needed for integer scale, for example you can write `scale 2` instead of `scale 2.0`.

<sup>Since: 0.1.7</sup> Scale below 0 and above 10 will now fail during config parsing. Scale was previously clamped to these values anyway.

```kdl
output "eDP-1" {
    scale 2.0
}
```

### `transform`

Rotate the output counter-clockwise.

Valid values are: `"normal"`, `"90"`, `"180"`, `"270"`, `"flipped"`, `"flipped-90"`, `"flipped-180"` and `"flipped-270"`.
Values with `flipped` additionally flip the output.

```kdl
output "HDMI-A-1" {
    transform "90"
}
```

### `position`

Set the position of the output in the global coordinate space.

This affects directional monitor actions like `focus-monitor-left`, and cursor movement.
The cursor can only move between directly adjacent outputs.

> [!NOTE]
> Output scale and rotation has to be taken into account for positioning: outputs are sized in logical, or scaled, pixels.
> For example, a 3840×2160 output with scale 2.0 will have a logical size of 1920×1080, so to put another output directly adjacent to it on the right, set its x to 1920.
> If the position is unset or results in an overlap, the output is instead placed automatically.

```kdl
output "HDMI-A-1" {
    position x=1280 y=0
}
```

#### Automatic Positioning

Niri repositions outputs from scratch every time the output configuration changes (which includes monitors disconnecting and connecting).
The following algorithm is used for positioning outputs.

1. Collect all connected monitors and their logical sizes.
1. Sort them by their name. This makes it so the automatic positioning does not depend on the order the monitors are connected. This is important because the connection order is non-deterministic at compositor startup.
1. Try to place every output with explicitly configured `position`, in order. If the output overlaps previously placed outputs, place it to the right of all previously placed outputs. In this case, niri will also print a warning.
1. Place every output without explicitly configured `position` by putting it to the right of all previously placed outputs.

### `variable-refresh-rate`

<sup>Since: 0.1.5</sup>

This flag enables variable refresh rate (VRR, also known as adaptive sync, FreeSync, or G-Sync), if the output supports it.

You can check whether an output supports VRR in `niri msg outputs`.

> [!NOTE]
> Some drivers have various issues with VRR.
>
> If the cursor moves at a low framerate with VRR, try setting the [`disable-cursor-plane` debug flag](./Configuration:-Debug-Options.md#disable-cursor-plane) and reconnecting the monitor.
>
> If a monitor is not detected as VRR-capable when it should, sometimes unplugging a different monitor fixes it.
>
> Some monitors will continuously modeset (flash black) with VRR enabled; I'm not sure if there's a way to fix it.

```kdl
output "HDMI-A-1" {
    variable-refresh-rate
}
```

<sup>Since: 0.1.9</sup> You can also set the `on-demand=true` property, which will only enable VRR when this output shows a window matching the `variable-refresh-rate` window rule.
This is helpful to avoid various issues with VRR, since it can be disabled most of the time, and only enabled for specific windows, like games or video players.

```kdl
output "HDMI-A-1" {
    variable-refresh-rate on-demand=true
}
```

### `focus-at-startup`

<sup>Since: 25.05</sup>

Focus this output by default when niri starts.

If multiple outputs with `focus-at-startup` are connected, they are prioritized in the order that they appear in the config.

When none of the connected outputs are explicitly `focus-at-startup`, niri will focus the first one sorted by name (same output sorting as used elsewhere in niri).

```kdl
// Focus HDMI-A-1 by default.
output "HDMI-A-1" {
    focus-at-startup
}

// ...if HDMI-A-1 wasn't connected, focus DP-2 instead.
output "DP-2" {
    focus-at-startup
}
```

### `background-color`

<sup>Since: 0.1.8</sup>

Set the background color that niri draws for workspaces on this output.
This is visible when you're not using any background tools like swaybg.

<sup>Until: 25.05</sup> The alpha channel for this color will be ignored.

<sup>Since: 25.11</sup> This setting is deprecated, set `background-color` in the [output `layout {}` block](#layout-config-overrides) instead.

```kdl
output "HDMI-A-1" {
    background-color "#003300"
}
```

### `backdrop-color`

<sup>Since: 25.05</sup>

Set the backdrop color that niri draws for this output.
This is visible between workspaces or in the overview.

The alpha channel for this color will be ignored.

```kdl
output "HDMI-A-1" {
    backdrop-color "#001100"
}
```

### `hot-corners`

<sup>Since: 25.11</sup>

Customize the hot corners for this output.
By default, hot corners [in the gestures settings](./Configuration:-Gestures.md#hot-corners) are used for all outputs.

Hot corners toggle the overview when you put your mouse at the very corner of a monitor.

`off` will disable the hot corners on this output, and writing specific corners will enable only those hot corners on this output.

```kdl
// Enable the bottom-left and bottom-right hot corners on HDMI-A-1.
output "HDMI-A-1" {
    hot-corners {
        bottom-left
        bottom-right
    }
}

// Disable the hot corners on DP-2.
output "DP-2" {
    hot-corners {
        off
    }
}
```

### Layout config overrides

<sup>Since: 25.11</sup>

You can customize layout settings for an output with a `layout {}` block:

```kdl
output "SomeCompany VerticalMonitor 1234" {
    transform "90"

    // Layout config overrides just for this output.
    layout {
        default-column-width { proportion 1.0; }

        // ...any other setting.
    }
}

output "SomeCompany UltrawideMonitor 1234" {
    // Narrower proportions and more presets for an ultrawide.
    layout {
        default-column-width { proportion 0.25; }

        preset-column-widths {
            proportion 0.2
            proportion 0.25
            proportion 0.5
            proportion 0.75
            proportion 0.8
        }
    }
}
```

It accepts all the same options as [the top-level `layout {}` block](./Configuration:-Layout.md).

In order to unset a flag, write it with `false`, e.g.:

```kdl
layout {
    // Enabled globally.
    always-center-single-column
}

output "eDP-1" {
    layout {
        // Unset on this output.
        always-center-single-column false
    }
}
```
