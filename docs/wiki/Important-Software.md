Since niri is not a complete desktop environment, you will very likely want to run the following software to make sure that other apps work fine.

### Notification Daemon

Many apps need one. For example, [mako](https://github.com/emersion/mako) works well. Use [a systemd setup](./Example-systemd-Setup.md) or [`spawn-at-startup`](./Configuration:-Miscellaneous.md#spawn-at-startup).

### Portals

These provide a cross-desktop API for apps to use for various things like file pickers or UI settings. Flatpak apps in particular require working portals.

Portals **require** [running niri as a session](./Getting-Started.md), which means through the `niri-session` script or from a display manager. You will want the following portals installed:

* `xdg-desktop-portal-gtk`: implements most of the basic functionality, this is the "default fallback portal".
* `xdg-desktop-portal-gnome`: required for screencasting support.
* `gnome-keyring`: implements the Secret portal, required for certain apps to work.

Then systemd should start them on-demand automatically. These particular portals are configured in `niri-portals.conf` which [must be installed](./Getting-Started.md#manual-installation) in the correct location.

Since we're using `xdg-desktop-portal-gnome`, Flatpak apps will read the GNOME UI settings. For example, to enable the dark style, run:

```
dconf write /org/gnome/desktop/interface/color-scheme '"prefer-dark"'
```

Note that if you're using the provided `resources/niri-portals.conf`, you also need to install the `nautilus` file manager in order for file chooser dialogues to work properly. This is necessary because xdg-desktop-portal-gnome uses nautilus as the file chooser by default starting from version 47.0.

If you do not want to install `nautilus` (say you use `nemo` instead), you can set `org.freedesktop.impl.portal.FileChooser=gtk;` in `niri-portals.conf` to use the GTK portal for file chooser dialogues.

> [!WARNING]
> Do not set the `GDK_BACKEND` environment variable globally as this will break the screencast portal.

### Authentication Agent

Required when apps need to ask for root permissions. Something like `plasma-polkit-agent` works fine. Start it [with systemd](./Example-systemd-Setup.md) or with [`spawn-at-startup`](./Configuration:-Miscellaneous.md#spawn-at-startup).

Note that to start `plasma-polkit-agent` with systemd on Fedora, you'll need to override its systemd service to add the correct dependency. Run:

```
systemctl --user edit --full plasma-polkit-agent.service
```

Then add `After=graphical-session.target`.

### Xwayland

To run X11 apps like Steam or Discord, you can use [xwayland-satellite].
Check [the Xwayland wiki page](./Xwayland.md) for instructions.

[xwayland-satellite]: https://github.com/Supreeeme/xwayland-satellite
