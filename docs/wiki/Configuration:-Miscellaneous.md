This page documents all top-level options that don't otherwise have dedicated pages.

Here are all of these options at a glance:

```kdl
spawn-at-startup "waybar"
spawn-at-startup "alacritty"
spawn-sh-at-startup "qs -c ~/source/qs/MyAwesomeShell"

prefer-no-csd

screenshot-path "~/Pictures/Screenshots/Screenshot from %Y-%m-%d %H-%M-%S.png"

environment {
    QT_QPA_PLATFORM "wayland"
    DISPLAY null
}

cursor {
    xcursor-theme "breeze_cursors"
    xcursor-size 48

    hide-when-typing
    hide-after-inactive-ms 1000
}

overview {
    zoom 0.5
    backdrop-color "#262626"

    workspace-shadow {
        // off
        softness 40
        spread 10
        offset x=0 y=10
        color "#00000050"
    }
}

xwayland-satellite {
    // off
    path "xwayland-satellite"
}

clipboard {
    disable-primary
}

hotkey-overlay {
    skip-at-startup
    hide-not-bound
}

config-notification {
    disable-failed
}
```

### `spawn-at-startup`

Add lines like this to spawn processes at niri startup.

`spawn-at-startup` accepts a path to the program binary as the first argument, followed by arguments to the program.

This option works the same way as the [`spawn` key binding action](./Configuration:-Key-Bindings.md#spawn), so please read about all its subtleties there.

```kdl
spawn-at-startup "waybar"
spawn-at-startup "alacritty"
```

Note that running niri as a systemd session supports xdg-desktop-autostart out of the box, which may be more convenient to use.
Thanks to this, apps that you configured to autostart in GNOME will also "just work" in niri, without any manual `spawn-at-startup` configuration.

### `spawn-sh-at-startup`

<sup>Since: 25.08</sup>

Add lines like this to run shell commands at niri startup.

The argument is a single string that is passed verbatim to `sh`.
You can use shell variables, pipelines, `~` expansion and everything else as expected.

See detailed description in the docs for the [`spawn-sh` key binding action](./Configuration:-Key-Bindings.md#spawn-sh).

```kdl
// Pass all arguments in the same string.
spawn-sh-at-startup "qs -c ~/source/qs/MyAwesomeShell"
```

### `prefer-no-csd`

This flag will make niri ask the applications to omit their client-side decorations.

If an application will specifically ask for CSD, the request will be honored.
Additionally, clients will be informed that they are tiled, removing some rounded corners.

With `prefer-no-csd` set, applications that negotiate server-side decorations through the xdg-decoration protocol will have focus ring and border drawn around them *without* a solid colored background.

> [!NOTE]
> Unlike most other options, changing `prefer-no-csd` will not entirely affect already running applications.
> It will make some windows rectangular, but won't remove the title bars.
> This mainly has to do with niri working around a [bug in SDL2](https://github.com/libsdl-org/SDL/issues/8173) that prevents SDL2 applications from starting.
>
> Restart applications after changing `prefer-no-csd` in the config to fully apply it.

```kdl
prefer-no-csd
```

### `screenshot-path`

Set the path where screenshots are saved.
A `~` at the front will be expanded to the home directory.

The path is formatted with `strftime(3)` to give you the screenshot date and time.

Niri will create the last folder of the path if it doesn't exist.

```kdl
screenshot-path "~/Pictures/Screenshots/Screenshot from %Y-%m-%d %H-%M-%S.png"
```

You can also set this option to `null` to disable saving screenshots to disk.

```kdl
screenshot-path null
```

### `environment`

Override environment variables for processes spawned by niri.

```kdl
environment {
    // Set a variable like this:
    // QT_QPA_PLATFORM "wayland"

    // Remove a variable by using null as the value:
    // DISPLAY null
}
```

Note that these variables do not propagate to the systemd global environment, so tools and applications started by systemd do not see them.
In particular, if you start a desktop shell like DankMaterialShell through systemd, then use its built-in application launcher, the apps won't see these environment variables.

If you want all processes to see the environment variables, you can set them in your login shell config instead (i.e. `~/.bash_profile`).
The `niri-session` shell script runs through the login shell and imports all environment variables to systemd before starting niri.
Keep in mind that all compositors will see variables set in the login shell, not just niri.

### `cursor`

Change the theme and size of the cursor as well as set the `XCURSOR_THEME` and `XCURSOR_SIZE` environment variables.

```kdl
cursor {
    xcursor-theme "breeze_cursors"
    xcursor-size 48
}
```

#### `hide-when-typing`

<sup>Since: 0.1.10</sup>

If set, hides the cursor when pressing a key on the keyboard.

> [!NOTE]
> This setting might interfere with games running in Wine in native Wayland mode that use mouselook, such as first-person games.
> If your character's point of view jumps down when you press a key and move the mouse simultaneously, try disabling this setting.

```kdl
cursor {
    hide-when-typing
}
```

#### `hide-after-inactive-ms`

<sup>Since: 0.1.10</sup>

If set, the cursor will automatically hide once this number of milliseconds passes since the last cursor movement.

```kdl
cursor {
    // Hide the cursor after one second of inactivity.
    hide-after-inactive-ms 1000
}
```

### `overview`

<sup>Since: 25.05</sup>

Settings for the [Overview](./Overview.md).

#### `zoom`

Control how much the workspaces zoom out in the overview.
`zoom` ranges from 0 to 0.75 where lower values make everything smaller.

```kdl
// Make workspaces four times smaller than normal in the overview.
overview {
    zoom 0.25
}
```

#### `backdrop-color`

Set the backdrop color behind workspaces in the overview.
The backdrop is also visible between workspaces when switching.

The alpha channel for this color will be ignored.

```kdl
// Make the backdrop light.
overview {
    backdrop-color "#777777"
}
```

You can also set the color per-output [in the output config](./Configuration:-Outputs.md#backdrop-color).

#### `workspace-shadow`

Control the shadow behind workspaces visible in the overview.

Settings here mirror the normal [`shadow` config in the layout section](./Configuration:-Layout.md#shadow), so check the documentation there.

Workspace shadows are configured for a workspace size normalized to 1080 pixels tall, then zoomed out together with the workspace.
Practically, this means that you'll want bigger spread, offset, and softness compared to window shadows.

```kdl
// Disable workspace shadows in the overview.
overview {
    workspace-shadow {
        off
    }
}
```

### `xwayland-satellite`

<sup>Since: 25.08</sup>

Settings for integration with [xwayland-satellite](https://github.com/Supreeeme/xwayland-satellite).

When a recent enough xwayland-satellite is detected, niri will create the X11 sockets and set `DISPLAY`, then automatically spawn `xwayland-satellite` when an X11 client tries to connect.
If Xwayland dies, niri will keep watching the X11 socket and restart `xwayland-satellite` as needed.
This is very similar to how built-in Xwayland works in other compositors.

`off` disables the integration: niri won't create an X11 socket and won't set the `DISPLAY` environment variable.

`path` sets the path to the `xwayland-satellite` binary.
By default, it's just `xwayland-satellite`, so it's looked up like any other non-absolute program name.

```kdl
// Use a custom build of xwayland-satellite.
xwayland-satellite {
    path "~/source/rs/xwayland-satellite/target/release/xwayland-satellite"
}
```

### `clipboard`

<sup>Since: 25.02</sup>

Clipboard settings.

Set the `disable-primary` flag to disable the primary clipboard (middle-click paste).
Toggling this flag will only apply to applications started afterward.

```kdl
clipboard {
    disable-primary
}
```

### `hotkey-overlay`

Settings for the "Important Hotkeys" overlay.

#### `skip-at-startup`

Set the `skip-at-startup` flag if you don't want to see the hotkey help at niri startup.

```kdl
hotkey-overlay {
    skip-at-startup
}
```

#### `hide-not-bound`

<sup>Since: 25.08</sup>

By default, niri will show the most important actions even if they aren't bound to any key, to prevent confusion.
Set the `hide-not-bound` flag if you want to hide all actions not bound to any key.

```kdl
hotkey-overlay {
    hide-not-bound
}
```

You can customize which binds the hotkey overlay shows using the [`hotkey-overlay-title` property](./Configuration:-Key-Bindings.md#custom-hotkey-overlay-titles).

### `config-notification`

<sup>Since: 25.08</sup>

Settings for the config created/failed notification.

Set the `disable-failed` flag to disable the "Failed to parse the config file" notification.
For example, if you have a custom one.

```kdl
config-notification {
    disable-failed
}
```
