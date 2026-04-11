### Overview

In this section you can configure input devices like keyboard and mouse, and some input-related options.

There's a section for each device type: `keyboard`, `touchpad`, `mouse`, `trackpoint`, `trackball`, `tablet`, `touchscreen`.
Settings in those sections will apply to every device of that type.
Currently, there's no way to configure specific devices individually (but that is planned).

All settings at a glance:

```kdl
input {
    keyboard {
        xkb {
            // layout "us"
            // variant "colemak_dh_ortho"
            // options "compose:ralt,ctrl:nocaps"
            // model ""
            // rules ""
            // file "~/.config/keymap.xkb"
        }

        // repeat-delay 600
        // repeat-rate 25
        // track-layout "global"
        numlock
    }

    touchpad {
        // off
        tap
        // dwt
        // dwtp
        // drag false
        // drag-lock
        natural-scroll
        // accel-speed 0.2
        // accel-profile "flat"
        // scroll-factor 1.0
        // scroll-factor vertical=1.0 horizontal=-2.0
        // scroll-method "two-finger"
        // scroll-button 273
        // scroll-button-lock
        // tap-button-map "left-middle-right"
        // click-method "clickfinger"
        // left-handed
        // disabled-on-external-mouse
        // middle-emulation

        // Touchpad gesture binds live in the main binds {} block using
        // the `TouchpadSwipe` trigger with `fingers=N direction="..."`
        // properties. This subblock only contains tuning parameters.
        // gestures {
        //     recognition-threshold 16.0
        //     gesture-progress-distance 40.0
        // }
    }

    mouse {
        // off
        // natural-scroll
        // accel-speed 0.2
        // accel-profile "flat"
        // scroll-factor 1.0
        // scroll-factor vertical=1.0 horizontal=-2.0
        // scroll-method "no-scroll"
        // scroll-button 273
        // scroll-button-lock
        // left-handed
        // middle-emulation
    }

    trackpoint {
        // off
        // natural-scroll
        // accel-speed 0.2
        // accel-profile "flat"
        // scroll-method "on-button-down"
        // scroll-button 273
        // scroll-button-lock
        // left-handed
        // middle-emulation
    }

    trackball {
        // off
        // natural-scroll
        // accel-speed 0.2
        // accel-profile "flat"
        // scroll-method "on-button-down"
        // scroll-button 273
        // scroll-button-lock
        // left-handed
        // middle-emulation
    }

    tablet {
        // off
        map-to-output "eDP-1"
        // left-handed
        // calibration-matrix 1.0 0.0 0.0 0.0 1.0 0.0
    }

    touchscreen {
        // off
        map-to-output "eDP-1"
        // natural-scroll
        // calibration-matrix 1.0 0.0 0.0 0.0 1.0 0.0

        // Touchscreen gesture binds live in the main binds {} block using
        // parameterized triggers like TouchSwipe fingers=3 direction="up",
        // TouchPinch fingers=4 direction="in", or TouchEdge edge="left".
        // This subblock only contains tuning parameters.
        gestures {
            // recognition-threshold 16.0
            // edge-threshold 20.0
            // pinch-threshold 30.0
            // pinch-ratio 2.0
            // pinch-sensitivity 1.0
            // pinch-progress-distance 100.0
            // finger-threshold-scale 1.5
            // gesture-progress-distance 200.0
            // rotation-threshold 15.0           // degrees
            // rotation-ratio 2.0
            // rotation-progress-distance 90.0   // degrees
        }
    }

    // disable-power-key-handling
    // warp-mouse-to-focus
    // focus-follows-mouse max-scroll-amount="0%"
    // workspace-auto-back-and-forth

    // mod-key "Super"
    // mod-key-nested "Alt"
}
```

### Keyboard

#### Layout

In the `xkb` section, you can set layout, variant, options, model and rules.
These are passed directly to libxkbcommon, which is also used by most other Wayland compositors.
See the `xkeyboard-config(7)` manual for more information.

```kdl
input {
    keyboard {
        xkb {
            layout "us"
            variant "colemak_dh_ortho"
            options "compose:ralt,ctrl:nocaps"
        }
    }
}
```

> [!TIP]
>
> <sup>Since: 25.02</sup>
>
> Alternatively, you can directly set a path to a .xkb file containing an xkb keymap.
> This overrides all other xkb settings.
>
> ```kdl
> input {
>     keyboard {
>         xkb {
>             file "~/.config/keymap.xkb"
>         }
>     }
> }
> ```

> [!NOTE]
>
> <sup>Since: 25.08</sup>
>
> If the `xkb` section is empty (like it is by default), niri will fetch xkb settings from systemd-localed at `org.freedesktop.locale1` over D-Bus.
> This way, for example, system installers can dynamically set the niri keyboard layout.
> You can see this layout in `localectl` and change it with `localectl set-x11-keymap`, for example:
>
> ```sh
> $ localectl set-x11-keymap "us" "" "colemak_dh_ortho" "compose:ralt,ctrl:nocaps"
> $ localectl
> System Locale: LANG=en_US.UTF-8
>                LC_NUMERIC=ru_RU.UTF-8
>                LC_TIME=ru_RU.UTF-8
>                LC_MONETARY=ru_RU.UTF-8
>                LC_PAPER=ru_RU.UTF-8
>                LC_MEASUREMENT=ru_RU.UTF-8
>     VC Keymap: us-colemak_dh_ortho
>    X11 Layout: us
>   X11 Variant: colemak_dh_ortho
>   X11 Options: compose:ralt,ctrl:nocaps
> ```
>
> By default, `localectl` will set the TTY keymap to the closest match of the XKB keymap.
> You can prevent that with a `--no-convert` flag, for example: `localectl set-x11-keymap --no-convert "us,ru"`.
>
> These settings are picked up by some other programs too, like GDM.

When using multiple layouts, niri can remember the current layout globally (the default) or per-window.
You can control this with the `track-layout` option.

- `global`: layout change is global for all windows.
- `window`: layout is tracked for each window individually.

```kdl
input {
    keyboard {
        track-layout "global"
    }
}
```

#### Repeat

Delay is in milliseconds before the keyboard repeat starts.
Rate is in characters per second.

```kdl
input {
    keyboard {
        repeat-delay 600
        repeat-rate 25
    }
}
```

#### Num Lock

<sup>Since: 25.05</sup>

Set the `numlock` flag to turn on Num Lock automatically at startup.

You might want to disable (comment out) `numlock` if you're using a laptop with a keyboard that overlays Num Lock keys on top of regular keys.

```kdl
input {
    keyboard {
        numlock
    }
}
```

### Pointing Devices

Most settings for the pointing devices are passed directly to libinput.
Other Wayland compositors also use libinput, so it's likely you will find the same settings there.
For flags like `tap`, omit them or comment them out to disable the setting.

A few settings are common between input devices:

- `off`: if set, no events will be sent from this device.

A few settings are common between `touchpad`, `mouse`, `trackpoint`, and `trackball`:

- `natural-scroll`: if set, inverts the scrolling direction.
- `accel-speed`: pointer acceleration speed, valid values are from `-1.0` to `1.0` where the default is `0.0`.
- `accel-profile`: can be `adaptive` (the default) or `flat` (disables pointer acceleration).
- `scroll-method`: when to generate scroll events instead of pointer motion events, can be `no-scroll`, `two-finger`, `edge`, or `on-button-down`.
  The default and supported methods vary depending on the device type.
- `scroll-button`: <sup>Since: 0.1.10</sup> the button code used for the `on-button-down` scroll method. You can find it in `libinput debug-events`.
- `scroll-button-lock`: <sup>Since: 25.08</sup> when enabled, the button does not need to be held down. Pressing once engages scrolling, pressing a second time disengages it, and double click acts as single click of the the underlying button.
- `left-handed`: if set, changes the device to left-handed mode.
- `middle-emulation`: emulate a middle mouse click by pressing left and right mouse buttons at once.

Settings specific to `touchpad`s:

- `tap`: tap-to-click.
- `dwt`: disable-when-typing.
- `dwtp`: disable-when-trackpointing.
- `drag`: <sup>Since: 25.05</sup> can be `true` or `false`, controls if tap-and-drag is enabled.
- `drag-lock`: <sup>Since: 25.02</sup> if set, lifting the finger off for a short time while dragging will not drop the dragged item. See the [libinput documentation](https://wayland.freedesktop.org/libinput/doc/latest/tapping.html#tap-and-drag).
- `tap-button-map`: can be `left-right-middle` or `left-middle-right`, controls which button corresponds to a two-finger tap and a three-finger tap.
- `click-method`: can be `button-areas` or `clickfinger`, changes the [click method](https://wayland.freedesktop.org/libinput/doc/latest/clickpad-softbuttons.html).
- `disabled-on-external-mouse`: do not send events while external pointer device is plugged in.

Settings specific to `touchpad` and `mouse`:

- `scroll-factor`: <sup>Since: 0.1.10</sup> scales the scrolling speed by this value.

    <sup>Since: 25.08</sup> You can also override horizontal and vertical scroll factor separately like so: `scroll-factor horizontal=2.0 vertical=-1.0`

Settings specific to `tablet` and `touchscreen`:

- `calibration-matrix`: set to six floating point numbers to change the calibration matrix. See the [`LIBINPUT_CALIBRATION_MATRIX` documentation](https://wayland.freedesktop.org/libinput/doc/latest/device-configuration-via-udev.html) for examples.
    - <sup>Since: 25.02</sup> for `tablet`
    - <sup>Since: 25.11</sup> for `touchscreen`

Settings specific to `touchscreen`:

- `natural-scroll`: <sup>Since: next</sup> if set, inverts the scrolling direction for touchscreen swipe gestures.
- `gestures {}`: <sup>Since: next</sup> tuning parameters for touchscreen gesture recognition.

> [!NOTE]
>
> Touchscreen gesture **binds** are configured in the main `binds {}` block using parameterized triggers like `TouchSwipe fingers=3 direction="up"`, `TouchPinch fingers=4 direction="in"`, or `TouchEdge edge="left"`. The `touchscreen { gestures { } }` subblock below only contains tuning parameters that affect *how* gestures are recognized, not *which* ones fire. See the [Gestures](./Gestures.md) wiki page for the full list of touchscreen gesture triggers.

The `touchscreen { gestures { } }` tuning parameters are:

- `recognition-threshold <float>`: distance in pixels fingers must move before a swipe gesture is recognized and starts firing events. Lower values feel more responsive but risk triggering on incidental finger drift. Default: `16.0`.
- `edge-threshold <float>`: distance in pixels from a screen edge within which a touch must start for it to count as an edge swipe (`TouchEdge edge="left|right|top|bottom"`, optionally with `zone=`). Touches beginning farther from the edge are treated as regular swipes. Default: `20.0`.
- `pinch-threshold <float>`: how far fingers must move together or apart (as total spread change in pixels) before niri classifies the gesture as a pinch rather than a swipe. Default: `30.0`.
- `pinch-ratio <float>`: ratio by which spread change must exceed linear swipe distance for a gesture to count as a pinch. Higher values make pinch detection stricter — the fingers really have to move apart/together rather than glide across the screen. Default: `2.0`.
- `pinch-sensitivity <float>`: multiplier mapping finger spread change (in screen pixels) to continuous pinch animation delta (e.g. overview open/close progress during a pinch). At the default of `1.0`, one pixel of finger spread change contributes one pixel to the underlying gesture accumulator, which is then divided by the target animation's threshold (e.g. 300 px for overview open/close). A typical 3-finger pinch travels 200–250 px of spread, which maps comfortably across the 0→1 overview progress range at `1.0`. Higher values make continuous pinch actions reach completion with less finger movement; lower values give finer control at the cost of needing more travel. Applies to **all** pinch-bound continuous actions, not just overview — the bind's own `sensitivity=` property is ignored for pinch gestures because raw spread-delta pixels need very different scaling from linear swipe distances. Default: `1.0`.
- `finger-threshold-scale <float>`: scaling applied to `recognition-threshold` for gestures with more than 3 fingers. The formula is `base * (1 + (fingers - 3) * (scale - 1))`, so with a base threshold of 16 and scale 1.5, a 4-finger gesture needs 24 px and a 5-finger gesture needs 32 px. Compensates for the extra movement spread that wider finger grips produce. Default: `1.5`.
- `gesture-progress-distance <float>`: pixels of finger movement required for IPC `GestureProgress` events on **swipe and edge** gestures to reach `progress = ±1.0`. Units are screen pixels. Tune this to make tagged external-app gestures (like sidebar drawers or scrubbers) feel right on your display. Default: `200.0`.
- `pinch-progress-distance <float>`: pixels of finger spread change for IPC `GestureProgress` events on **pinch** gestures to reach `progress = ±1.0`. Signed: positive for pinch-out (spread growing), negative for pinch-in. Pinch spread changes are usually smaller than linear swipe distances, so this defaults lower than `gesture-progress-distance`. Default: `100.0`.
- `rotation-threshold <float>`: minimum cumulative rotation (in **degrees**) before a gesture classifies as a rotation rather than a pinch or swipe. Default: `15.0`. Rotation detection is currently an early proof of concept — see the warning in the [Rotation Gestures](./Gestures.md#rotation-gestures) section.
- `rotation-ratio <float>`: ratio by which the rotation arc length (`|cumulative_rotation| * cluster_radius`) must dominate both swipe distance and spread change for a gesture to classify as a rotation. Lower values make rotation detection more permissive (easier to trigger on incidental twist); higher values make it stricter. Default: `0.5`.
- `rotation-progress-distance <float>`: degrees of cumulative rotation for IPC `GestureProgress` events on **rotation** gestures to reach `progress = ±1.0`. Signed with the on-screen convention: positive = counter-clockwise on screen, negative = clockwise. Default: `90.0`.

Example:

```kdl
input {
    touchscreen {
        gestures {
            recognition-threshold 26.0
            edge-threshold 30.0
            pinch-sensitivity 1.0
            gesture-progress-distance 200.0
            pinch-progress-distance 100.0
        }
    }
}
```

### Touchpad Gesture Tuning

<sup>Since: next</sup>

The `touchpad { gestures { } }` subblock contains tuning parameters for touchpad swipe recognition. Like touchscreen, the actual gesture binds (`TouchpadSwipe fingers=N direction="..."`) live in the main `binds {}` block.

- `recognition-threshold <float>`: distance in libinput delta units that fingers must move before a swipe gesture is recognized. These units are acceleration-adjusted and not directly comparable to touchscreen pixels. Default: `16.0`.
- `gesture-progress-distance <float>`: libinput delta units of finger movement required for IPC `GestureProgress` events to reach `progress = 1.0`. Because libinput acceleration curves are nonlinear, the same physical swipe can produce different delta magnitudes depending on speed — this value is **not** directly comparable to the touchscreen `gesture-progress-distance`. Default: `40.0`.

Example:

```kdl
input {
    touchpad {
        gestures {
            recognition-threshold 16.0
            gesture-progress-distance 40.0
        }
    }
}
```

Tablets and touchscreens are absolute pointing devices that can be mapped to a specific output like so:

```kdl
input {
    tablet {
        map-to-output "eDP-1"
    }

    touchscreen {
        map-to-output "eDP-1"
    }
}
```

Valid output names are the same as the ones used for output configuration.

<sup>Since: 0.1.7</sup> When a tablet is not mapped to any output, it will map to the union of all connected outputs, without aspect ratio correction.

### General Settings

These settings are not specific to a particular input device.

#### `disable-power-key-handling`

By default, niri will take over the power button to make it sleep instead of power off.
Set this if you would like to configure the power button elsewhere (i.e. `logind.conf`).

```kdl
input {
    disable-power-key-handling
}
```

#### `warp-mouse-to-focus`

Makes the mouse warp to newly focused windows.

Does not make the cursor visible if it had been hidden.

```kdl
input {
    warp-mouse-to-focus
}
```

By default, the cursor warps *separately* horizontally and vertically.
I.e. if moving the mouse only horizontally is enough to put it inside the newly focused window, then the mouse will move only horizontally, and not vertically.

<sup>Since: 25.05</sup> You can customize this with the `mode` property.

- `mode="center-xy"`: warps by both X and Y coordinates together.
So if the mouse was anywhere outside the newly focused window, it will warp to the center of the window.
- `mode="center-xy-always"`: warps by both X and Y coordinates together, even if the mouse was already somewhere inside the newly focused window.

```kdl
input {
    warp-mouse-to-focus mode="center-xy"
}
```

#### `focus-follows-mouse`

Focuses windows and outputs automatically when moving the mouse over them.

```kdl
input {
    focus-follows-mouse
}
```

<sup>Since: 0.1.8</sup> You can optionally set `max-scroll-amount`.
Then, focus-follows-mouse won't focus a window if it will result in the view scrolling more than the set amount.
The value is a percentage of the working area width.

```kdl
input {
    // Allow focus-follows-mouse when it results in scrolling at most 10% of the screen.
    focus-follows-mouse max-scroll-amount="10%"
}
```

```kdl
input {
    // Allow focus-follows-mouse only when it will not scroll the view.
    focus-follows-mouse max-scroll-amount="0%"
}
```

#### `workspace-auto-back-and-forth`

Normally, switching to the same workspace by index twice will do nothing (since you're already on that workspace).
If this flag is enabled, switching to the same workspace by index twice will switch back to the previous workspace.

Niri will correctly switch to the workspace you came from, even if workspaces were reordered in the meantime.

```kdl
input {
    workspace-auto-back-and-forth
}
```

#### `mod-key`, `mod-key-nested`

<sup>Since: 25.05</sup>

Customize the `Mod` key for [key bindings](./Configuration:-Key-Bindings.md).
Only valid modifiers are allowed, e.g. `Super`, `Alt`, `Mod3`, `Mod5`, `Ctrl`, `Shift`.

By default, `Mod` is equal to `Super` when running niri on a TTY, and to `Alt` when running niri as a nested winit window.

> [!NOTE]
> There are a lot of default bindings with Mod, none of them "make it through" to the underlying window.
> You probably don't want to set `mod-key` to Ctrl or Shift, since Ctrl is commonly used for app hotkeys, and Shift is used for, well, regular typing.

```kdl
// Switch the mod keys around: use Alt normally, and Super inside a nested window.
input {
    mod-key "Alt"
    mod-key-nested "Super"
}
```
