### How to disable client-side decorations/make windows rectangular?

Uncomment the [`prefer-no-csd` setting](./Configuration:-Miscellaneous.md#prefer-no-csd) at the top level of the config, and then restart your apps.
Then niri will ask windows to omit client-side decorations, and also inform them that they are being tiled (which makes some windows rectangular, even if they cannot omit the decorations).

Note that currently this will prevent edge window resize handles from showing up.
You can still resize windows by holding <kbd>Mod</kbd> and the right mouse button.

### Why are transparent windows tinted? / Why is the border/focus ring showing up through semitransparent windows?

Uncomment the [`prefer-no-csd` setting](./Configuration:-Miscellaneous.md#prefer-no-csd) at the top level of the config, and then restart your apps.
Niri will draw focus rings and borders *around* windows that agree to omit their client-side decorations.

By default, focus ring and border are rendered as a solid background rectangle behind windows.
That is, they will show up through semitransparent windows.
This is because windows using client-side decorations can have an arbitrary shape.

You can also override this behavior with the [`draw-border-with-background` window rule](./Configuration:-Window-Rules.md#draw-border-with-background).

### How to enable rounded corners for all windows?

Put this window rule in your config:

```kdl
window-rule {
    geometry-corner-radius 12
    clip-to-geometry true
}
```

For more information, check the [`geometry-corner-radius` window rule](./Configuration:-Window-Rules.md#geometry-corner-radius).

### How to hide the "Important Hotkeys" pop-up at the start?

Put this into your config:

```kdl
hotkey-overlay {
    skip-at-startup
}
```

### How to run X11 apps like Steam or Discord?

To run X11 apps, you can use [xwayland-satellite](https://github.com/Supreeeme/xwayland-satellite).
Check [the Xwayland wiki page](./Xwayland.md) for instructions.

Keep in mind that you can run many Electron apps such as VSCode or Discord natively on Wayland by passing the right flags, as described [here](./Application-Issues.md#electron-applications).

### Why doesn't niri integrate Xwayland like other compositors?

A combination of factors:

- Integrating Xwayland is quite a bit of work, as the compositor needs to implement parts of an X11 window manager.
- You need to appease the X11 ideas of windowing, whereas for niri I want to have the best code for Wayland.
- niri doesn't have a good global coordinate system required by X11.
- You tend to get an endless stream of X11 bugs that take further time and effort away from other tasks.
- There aren't actually that many X11-only clients nowadays, and xwayland-satellite takes perfect care of most of those.
- niri isn't a Big Serious Desktop Environment which Must Support All Use Cases (and is Backed By Some Corporation).

All in all, the situation works out in favor of avoiding Xwayland integration.

<sup>Since: 25.08</sup> niri has seamless built-in xwayland-satellite integration that by and large works as well as built-in Xwayland in other compositors, solving the hurdle of having to set it up manually.

I wouldn't be too surprised if, down the road, xwayland-satellite becomes the standard way of integrating Xwayland into new compositors, since it takes on the bulk of the annoying work, and isolates the compositor from misbehaving clients.

### Can I enable blur behind semitransparent windows?

Not yet, follow/upvote [this issue](https://github.com/YaLTeR/niri/issues/54).

There's also [a PR](https://github.com/YaLTeR/niri/pull/1634) adding blur to niri which you can build and run manually.
Keep in mind that it's an experimental implementation that may have problems and performance concerns.

### Can I make a window sticky / pinned / always on top / appear on all workspaces?

Not yet, follow/upvote [this issue](https://github.com/YaLTeR/niri/issues/932).

You can emulate this with a script that uses the niri IPC.
For example, [nirius](https://git.sr.ht/~tsdh/nirius) seems to have this feature (`toggle-follow-mode`).

### How do I make the Bitwarden window in Firefox open as floating?

Firefox seems to first open the Bitwarden window with a generic Firefox title, and only later change the window title to Bitwarden, so you can't effectively target it with an `open-floating` window rule.

You'll need to use a script, for example [this one](https://github.com/YaLTeR/niri/discussions/1599) or other ones (search niri issues and discussions for Bitwarden).

### Can I open a window directly in the current column / in the same column as another window?

No, but you can script the behavior you want with the [niri IPC](./IPC.md).
Listen to the event stream for a new window opening, then call an action like `consume-or-expel-window-left`.

Adding this directly to niri is challenging:

- The act of "opening a window directly in some column" by itself is quite involved. Niri will have to compute the exact initial window size provided how other windows in a column would resize in response. This logic exists, but it isn't directly pluggable to the code computing a size for a new window. Then, it'll need to handle all sorts of edge cases like the column disappearing, or new windows getting added to the column, before the target window had a chance to appear.
- How do you indicate if a new window should spawn in an existing column (and in which one), as opposed to a new column? Different people seem to have different needs here (including very complex rules based on parent PID, etc.), and it's very unclear design-wise what kind of (simple) setting is actually needed and would be useful. See also https://github.com/YaLTeR/niri/discussions/1125.

### Why does moving the mouse against a monitor edge focus the next window, but only sometimes?

This can happen with [`focus-follows-mouse`](./Configuration:-Input.md#focus-follows-mouse).
When using client-side decorations, windows are supposed to have some margins outside their geometry for the mouse resizing handles.
These margins "peek out" of the monitor edges since they're outside the window geometry, and `focus-follows-mouse` triggers when the mouse crosses them.

It doesn't always happen:

- Some toolkits don't put resize handles outside the window geometry. Then there's no input area outside, so nowhere for `focus-follows-mouse` to trigger.
- If the current window has its own margin for resizing, and it extends all the way to the monitor edge, then `focus-follows-mouse` won't trigger because the mouse will never leave the current window.

To fix this, you can:

- Use `focus-follows-mouse max-scroll-amount="0%"`, which will prevent `focus-follows-mouse` from triggering when it would cause scrolling.
- Set `prefer-no-csd` which will generally cause clients to remove those resizing margins.

### How do I recover from a dead screen locker / from a red screen?

When your screen locker dies, you will be left with a red screen.
This is niri's locked session background.

You can recover from this by spawning a new screen locker.
One way is to switch to a different TTY (with a shortcut like <kbd>Ctrl</kbd><kbd>Alt</kbd><kbd>F3</kbd>) and spawning a screen locker to niri's Wayland display, e.g. `WAYLAND_DISPLAY=wayland-1 swaylock`.

Another way is to set `allow-when-locked=true` on your screen locker bind, then you can press it on the red screen to get a fresh screen locker.
```kdl
binds {
    Super+Alt+L allow-when-locked=true { spawn "swaylock"; }
}
```

### How do I change output configuration based on connected monitors?

If you require different output configurations depending on what outputs are connected then you can use [Kanshi](https://gitlab.freedesktop.org/emersion/kanshi).

Kanshi has its own simple configuration and communicates with niri via IPC. You may want to launch kanshi from the niri config.kdl e.g. `spawn-at-startup "/usr/bin/kanshi"`

For example, if you wish to scale your laptop display differently when an external monitor is connected, you might use a Kanshi config like this:
```
profile {
	output eDP-1 enable scale 1.0
}

profile { 
	output HDMI-A-1 enable scale 1.0 position 0,0
	output eDP-1 enable scale 1.25 position 1920,0
}
```

### Why does Firefox or Thunderbird have 1 px smaller border?

They draw their own 1 px dark border around the window, which obscures one pixel of niri's border.
If you don't like this, set the [`clip-to-geometry true` window rule](./Configuration:-Window-Rules.md#clip-to-geometry).
