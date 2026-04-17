### Electron applications

Electron-based applications can run directly on Wayland, but it's not the default.

For Electron ≥ 39, you can use the command-line flag if the app does not default to Wayland:
```
--ozone-platform=wayland
```

For Electron < 39, you can set an environment variable:
```kdl
environment {
    ELECTRON_OZONE_PLATFORM_HINT "auto"
}
```

For Electron ≤ 28, you need to pass command-line flags to the target application:
```
--enable-features=UseOzonePlatform --ozone-platform-hint=auto
```

If the application has a [desktop entry](https://specifications.freedesktop.org/menu-spec/latest/menu-add-example.html), you can put the command-line arguments into the `Exec` section.

### VSCode

If you're having issues with some VSCode hotkeys, try starting `Xwayland` and setting the `DISPLAY=:0` environment variable for VSCode.
That is, still running VSCode with the Wayland backend, but with `DISPLAY` set to a running Xwayland instance.
Apparently, VSCode currently unconditionally queries the X server for a keymap.

### JetBrains IDEs

JetBrains IDEs can run directly on Wayland, but it's not the default.

For JetBrainsRuntime > 17, you can set the flag `-Dawt.toolkit.name=WLToolkit` inside of `help -> edit custom vm options -> add`.

### WezTerm

> [!NOTE]
> Both of these issues seem to be fixed in the nightly build of WezTerm.

There's [a bug](https://github.com/wezterm/wezterm/issues/4708) in WezTerm that it waits for a zero-sized Wayland configure event, so its window never shows up in niri. To work around it, put this window rule in the niri config (included in the default config):

```kdl
window-rule {
    match app-id=r#"^org\.wezfurlong\.wezterm$"#
    default-column-width {}
}
```

This empty default column width lets WezTerm pick its own initial width which makes it show up properly.

There's [another bug](https://github.com/wezterm/wezterm/issues/6472) in WezTerm that causes it to choose a wrong size when it's in a tiled state, and prevent resizing it.
Niri puts windows in the tiled state with [`prefer-no-csd`](./Configuration:-Miscellaneous.md#prefer-no-csd).
So if you hit this problem, comment out `prefer-no-csd` in the niri config and restart WezTerm.

### Ghidra

Some Java apps like Ghidra can show up blank under xwayland-satellite.
To fix this, run them with the `_JAVA_AWT_WM_NONREPARENTING=1` environment variable.

### Zen Browser

For some reason, DMABUF screencasts are disabled in the Zen Browser, so screencasting doesn't work out of the box on niri.
To fix it, open `about:config` and set `widget.dmabuf.force-enabled` to `true`.

### GTK 4 dead keys / Compose

GTK 4.20 [stopped](https://gitlab.gnome.org/GNOME/gtk/-/merge_requests/8556) handling dead keys and Compose on its own on Wayland.
To make them work, either run an IME like IBus or Fcitx5, or set the `GTK_IM_MODULE=simple` environment variable.

```kdl
environment {
    GTK_IM_MODULE "simple"
}
```

Note that the niri environment config does not propagate to apps and shells started by systemd, for example to DankMaterialShell and its application launcher.
You can set the variable in your login shell config (i.e. `~/.bash_profile`) instead, though keep in mind that then it will be set for all compositors, not just niri.

### Fullscreen games

Some video games, both Linux-native and on Wine, have various issues when using non-stacking desktop environments.
Most of these can be avoided with Valve's [gamescope](https://github.com/ValveSoftware/gamescope), for example:

```sh
gamescope -f -w 1920 -h 1080 -W 1920 -H 1080 --force-grab-cursor --backend sdl -- <game>
```

This command will run *<game>* in 1080p fullscreen—make sure to replace the width and height values to match your desired resolution.
`--force-grab-cursor` forces gamescope to use relative mouse movement which prevents the cursor from escaping the game's window on multi-monitor setups.
Note that `--backend sdl` is currently also required as gamescope's default Wayland backend doesn't lock the cursor properly (possibly related to https://github.com/ValveSoftware/gamescope/issues/1711).

Steam users should use gamescope through a game's [launch options](https://help.steampowered.com/en/faqs/view/7D01-D2DD-D75E-2955) by replacing the game executable with `%command%`.
Other game launchers such as [Lutris](https://lutris.net/) have their own ways of setting gamescope options.

Running X11-based games with this method doesn't require Xwayland as gamescope creates its own Xwayland server.
You can run Wayland-native games as well by passing `--expose-wayland` to gamescope, therefore eliminating X11 from the equation.

### Steam

On some systems, Steam will show a fully black window.
To fix this, navigate to Settings -> Interface (via Steam's tray icon, or by blindly finding the Steam menu at the top left of the window), then **disable** GPU accelerated rendering in web views.
Restart Steam and it should now work fine.

If you do not want to disable GPU accelerated rendering you can instead try to pass the launch argument `-system-composer` instead.

Steam notifications don't run through the standard notification daemon and show up as floating windows in the center of the screen.
You can move them to a more convenient location by adding a window rule in your niri config:

```kdl
window-rule {
    match app-id="steam" title=r#"^notificationtoasts_\d+_desktop$"#
    default-floating-position x=10 y=10 relative-to="bottom-right"
}
```

### Waybar and other GTK 3 components

If you have rounded corners on your Waybar and they show up with black pixels in the corners, then set your Waybar opacity to 0.99, which should fix it.

GTK 3 seems to have a bug where it reports a surface as fully opaque even if it has rounded corners.
This leads to niri filling the transparent pixels inside the corners with black.

Setting the surface opacity to something below 1 fixes the problem because then GTK no longer reports the surface as opaque.
