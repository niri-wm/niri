## Screen readers

<sup>Since: 25.08</sup>

Niri has basic support for screen readers (specifically, [Orca](https://orca.gnome.org)) when running as a full desktop session, i.e. you need to start niri through a display manager or through `niri-session`.
To avoid conflicts with an already running compositor, niri won't expose accessibility interfaces when started as a nested window, or as a plain `/usr/bin/niri` on a TTY.

We implement the `org.freedesktop.a11y.KeyboardMonitor` D-Bus interface for Orca to listen and grab keyboard keys, and we expose the main niri UI elements via [AccessKit](https://accesskit.dev).
Specifically, niri will announce:

- workspace switching, for example it'll say "Workspace 2" when you switch to the second workspace;
- the exit confirmation dialog (appears on <kbd>Super</kbd><kbd>Shift</kbd><kbd>E</kbd> by default);
- <sup>Since: 25.11</sup> niri has an <kbd>Alt</kbd><kbd>Tab</kbd> window switcher where it will announce the selected window title;
- entering the screenshot UI and the overview (niri will say when these are focused, nothing else for now);
- whenever a config parse error occurs;
- the important hotkeys list (for now, as one big announcement without tab navigation; appears on <kbd>Super</kbd><kbd>Shift</kbd><kbd>/</kbd> by default).

Here's a demo video, watch with sound on.

<video controls src="https://github.com/user-attachments/assets/afceba6f-79f1-47ec-b859-a0fcb7f8eae3">

https://github.com/user-attachments/assets/afceba6f-79f1-47ec-b859-a0fcb7f8eae3

</video>

Make sure [Xwayland](./Xwayland.md) works, then run `orca`.
The default config binds <kbd>Super</kbd><kbd>Alt</kbd><kbd>S</kbd> to toggle Orca, which is the standard key binding.

Note that there are some limitations:

- We don't have a bind to move focus to layer-shell panels. This is not hard to add, but it would be good to have some consensus or prior art with LXQt/Xfce on how exactly this should work.
- You need to have a screen connected and enabled. Without a screen, niri won't give focus any window. This makes sense for sighted users, and I'm not entirely sure what makes the most sense for accessibility purposes (maybe, it'd be better solved with virtual monitors).
- You need working EGL (hardware acceleration).
- We don't have screen curtain functionality yet.

If you're shipping niri and would like to make it work better for screen readers out of the box, consider the following changes to the default niri config:

- Change the default terminal from Alacritty to one that supports screen readers. For example, [GNOME Console](https://gitlab.gnome.org/GNOME/console) or [GNOME Terminal](https://gitlab.gnome.org/GNOME/gnome-terminal) should work well.
- Change the default application launcher and screen locker to ones that support screen readers. For example, [xfce4-appfinder](https://docs.xfce.org/xfce/xfce4-appfinder/start) is an accessible launcher. Suggestions welcome! Likely, something GTK-based will work fine.
- Add some [`spawn-at-startup`](./Configuration:-Miscellaneous.md#spawn-at-startup) command that plays a sound which will indicate to users that niri has finished loading.
- Add `spawn-at-startup "orca"` to run Orca automatically at niri startup.

## Desktop zoom

There's no built-in zoom yet, but you can use third-party utilities like [wooz](https://github.com/negrel/wooz).
