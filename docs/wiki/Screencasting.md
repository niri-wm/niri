### Overview

The primary screencasting interface that niri offers is through portals and pipewire.
It is supported by [OBS], Firefox, Chromium, Electron, Telegram, and other apps.
You can screencast both monitors and individual windows.

In order to use it, you need a working D-Bus session, pipewire, `xdg-desktop-portal-gnome`, and [running niri as a session](./Getting-Started.md) (i.e. through `niri-session` or from a display manager).
On widely used distros this should all "just work".

Alternatively, you can use tools that rely on the `wlr-screencopy` protocol, which niri also supports.

There are several features in niri designed for screencasting.
Let's take a look!

### Block out windows

You can block out specific windows from screencasts, replacing them with solid black rectangles.
This can be useful for password managers or messenger windows, etc.

![Screenshot showing a window visible normally, but blocked out on OBS.](./img/block-out-from-screencast.png)

This is controlled through the `block-out-from` window rule, for example:

```kdl
// Block out password managers from screencasts.
window-rule {
    match app-id=r#"^org\.keepassxc\.KeePassXC$"#
    match app-id=r#"^org\.gnome\.World\.Secrets$"#

    block-out-from "screencast"
}
```

You can similarly block out layer surfaces, using a layer rule:

```kdl
// Block out mako notifications from screencasts.
layer-rule {
    match namespace="^notifications$"

    block-out-from "screencast"
}
```

Check [the corresponding wiki section](./Configuration:-Window-Rules.md#block-out-from) for more details and examples.

### Dynamic screencast target

<sup>Since: 25.05</sup>

Niri provides a special screencast stream that you can change dynamically.
It shows up as "niri Dynamic Cast Target" in the screencast window dialog.

![Screencast dialog showing niri Dynamic Cast Target.](https://github.com/user-attachments/assets/e236ce74-98ec-4f3a-a99b-29ac1ff324dd)

When you select it, it will start as an empty, transparent video stream.
Then, you can use the following binds to change what it shows:

- `set-dynamic-cast-window` to cast the focused window.
- `set-dynamic-cast-monitor` to cast the focused monitor.
- `clear-dynamic-cast-target` to go back to an empty stream.

You can also use these actions from the command line, for example to interactively pick which window to cast:

```sh
$ niri msg action set-dynamic-cast-window --id $(niri msg --json pick-window | jq .id)
```

<video controls src="https://github.com/user-attachments/assets/c617a9d6-7d5e-4f1f-b8cc-9301182d9634">

https://github.com/user-attachments/assets/c617a9d6-7d5e-4f1f-b8cc-9301182d9634

</video>

If the cast target disappears (e.g. the target window closes), the stream goes back to empty.

All dynamic casts share the same target, but new ones start out empty until the next time you change it (to avoid surprises and sharing something sensitive by mistake).

### Indicate screencasted windows

<sup>Since: 25.02</sup>

The [`is-window-cast-target=true` window rule](./Configuration:-Window-Rules.md#is-window-cast-target) matches windows targeted by an ongoing window screencast.
You use it with a special border color to clearly indicate screencasted windows.

This also works for windows targeted by dynamic screencasts.
However, it will not work for windows that just happen to be visible in a full-monitor screencast.

```kdl
// Indicate screencasted windows with red colors.
window-rule {
    match is-window-cast-target=true

    focus-ring {
        active-color "#f38ba8"
        inactive-color "#7d0d2d"
    }

    border {
        inactive-color "#7d0d2d"
    }

    shadow {
        color "#7d0d2d70"
    }

    tab-indicator {
        active-color "#f38ba8"
        inactive-color "#7d0d2d"
    }
}
```

Example:

![Screencasted window indicated with a red border and shadow.](https://github.com/user-attachments/assets/375b381e-3a87-4e94-8676-44404971d893)

### Windowed (fake/detached) fullscreen

<sup>Since: 25.05</sup>

When screencasting browser-based presentations like Google Slides, you usually want to hide the browser UI, which requires making the browser fullscreen.
This is not always convenient, for example if you have an ultrawide monitor, or just want to leave the browser as a smaller window, without taking up an entire monitor.

The `toggle-windowed-fullscreen` bind helps with this.
It tells the app that it went fullscreen, while in reality leaving it as a normal window that you can resize and put wherever you want.

```kdl
binds {
    Mod+Ctrl+Shift+F { toggle-windowed-fullscreen; }
}
```

Keep in mind that not all apps react to fullscreening, so it may sometimes look as if the bind did nothing.

Here's an example showing a windowed-fullscreen Google Slides [presentation](https://youtu.be/Kmz8ODolnDg), along with the presenter view and a meeting app:

![Windowed Google Slides presentation, another window showing the presenter view, and another window showing Zoom UI casting the presentation.](https://github.com/user-attachments/assets/b2b49eea-f5a0-4c0a-b537-51fd1949a59d)

### Screen mirroring

For presentations it can be useful to mirror an output to another.
Currently, niri doesn't have built-in output mirroring, but you can use a third-party tool [`wl-mirror`](https://github.com/Ferdi265/wl-mirror) that mirrors an output to a window.
Note that the command below requires [`jq`](https://jqlang.org/download/) to be installed.
```kdl
binds {
    Mod+P repeat=false { spawn-sh "wl-mirror $(niri msg --json focused-output | jq -r .name)"; }
}
```
Focus the output you want to mirror, press <kbd>Mod</kbd><kbd>P</kbd> and move the `wl-mirror` window to the target output.
Finally, fullscreen the `wl-mirror` window (by default, <kbd>Mod</kbd><kbd>Shift</kbd><kbd>F</kbd>).

[OBS]: https://obsproject.com/
