## Using xwayland-satellite

<sup>Since: 25.08</sup> Niri integrates with [xwayland-satellite](https://github.com/Supreeeme/xwayland-satellite) out of the box.
Ensure xwayland-satellite >= 0.7 is installed and available in `$PATH`.
With no further configuration, niri will create X11 sockets on disk, export `$DISPLAY`, and spawn xwayland-satellite on-demand when an X11 client connects.
If xwayland-satellite dies, niri will automatically restart it.

If you had a custom config which manually started `xwayland-satellite` and set `$DISPLAY`, you should remove those customizations for the automatic integration to work.

To check that the integration works, verify that the niri output says something like `listening on X11 socket: :0`:

```sh
$ journalctl --user-unit=niri -b
systemd[2338]: Starting niri.service - A scrollable-tiling Wayland compositor...
niri[2474]: 2025-08-29T04:07:40.043402Z  INFO niri: starting version 25.05.1 (0.0.git.2345.d9833fc1)
(...)
niri[2474]: 2025-08-29T04:07:40.690512Z  INFO niri: listening on Wayland socket: wayland-1
niri[2474]: 2025-08-29T04:07:40.690520Z  INFO niri: IPC listening on: /run/user/1000/niri.wayland-1.2474.sock
niri[2474]: 2025-08-29T04:07:40.700137Z  INFO niri: listening on X11 socket: :0
systemd[2338]: Started niri.service - A scrollable-tiling Wayland compositor.
$ echo $DISPLAY
:0
```

![xwayland-satellite running Steam and Half-Life.](https://github.com/user-attachments/assets/57db8f96-40d4-4621-a389-373c169349a4)

We're using xwayland-satellite rather than Xwayland directly because [X11 is very cursed](./FAQ.md#why-doesnt-niri-integrate-xwayland-like-other-compositors).
xwayland-satellite takes on the bulk of the work dealing with the X11 peculiarities from us, giving niri normal Wayland windows to manage.

xwayland-satellite works well with most applications: Steam, games, Discord, even more exotic things like Ardour with wine Windows VST plugins.
However, X11 apps that want to position windows or bars at specific screen coordinates won't behave correctly and will need a nested compositor to run.
See sections below for how to do that.

## Using the labwc Wayland compositor

[Labwc](https://github.com/labwc/labwc) is a traditional stacking Wayland compositor with Xwayland.
You can run it as a window, then run X11 apps inside.

1. Install labwc from your distribution packages.
1. Run it inside niri with the `labwc` command.
It will open as a new window.
1. Run an X11 application on the X11 DISPLAY that it provides, e.g. `env DISPLAY=:0 glxgears`

![Labwc running X11 apps.](https://github.com/user-attachments/assets/aecbcecb-f0cb-4909-867f-09d34b5a2d7e)

## Directly running Xwayland in rootful mode

This method involves invoking XWayland directly and running it as its own window, it also requires an extra X11 window manager running inside it.

![Xwayland running in rootful mode.](https://github.com/niri-wm/niri/assets/1794388/b64e96c4-a0bb-4316-94a0-ff445d4c7da7)

Here's how you do it:

1. Run `Xwayland` (just the binary on its own without flags).
This will spawn a black window which you can resize and fullscreen (with Mod+Shift+F) for convenience.
On older Xwayland versions the window will be screen-sized and non-resizable.
1. Run some X11 window manager in there, e.g. `env DISPLAY=:0 i3`.
This way you can manage X11 windows inside the Xwayland instance.
1. Run an X11 application there, e.g. `env DISPLAY=:0 flatpak run com.valvesoftware.Steam`.

With fullscreen game inside a fullscreen Xwayland you get pretty much a normal gaming experience.

> [!TIP]
> If you don't run an X11 window manager, Xwayland will close and re-open its window every time all X11 windows close and a new one opens.
> To prevent this, start an X11 WM inside as mentioned above, or open some other long-running X11 window.

One caveat is that currently rootful Xwayland doesn't seem to share clipboard with the compositor.
For textual data you can do it manually using [wl-clipboard](https://github.com/bugaevc/wl-clipboard), for example:

- `env DISPLAY=:0 xsel -ob | wl-copy` to copy from Xwayland to niri clipboard
- `wl-paste -n | env DISPLAY=:0 xsel -ib` to copy from niri to Xwayland clipboard

You can also bind these to hotkeys if you want:

```
binds {
    Mod+Shift+C { spawn "sh" "-c" "env DISPLAY=:0 xsel -ob | wl-copy"; }
    Mod+Shift+V { spawn "sh" "-c" "wl-paste -n | env DISPLAY=:0 xsel -ib"; }
}
```

## Using xwayland-run to run Xwayland

[xwayland-run] is a helper utility to run an X11 client within a dedicated Xwayland rootful server.
It takes care of starting Xwayland, setting the X11 DISPLAY environment variable, setting up xauth and running the specified X11 client using the newly started Xwayland instance.
When the X11 client terminates, xwayland-run will automatically close the dedicated Xwayland server.

It works like this:

```
xwayland-run <Xwayland arguments> -- your-x11-app <X11 app arguments>
```

For example:

```
xwayland-run -geometry 800x600 -fullscreen -- wine wingame.exe
```

## Using the Cage Wayland compositor

It is also possible to run the X11 application in [Cage](https://github.com/cage-kiosk/cage), which runs a nested Wayland session which also supports Xwayland, where the X11 application can run in.

Compared to the Xwayland rootful method, this does not require running an extra X11 window manager, and can be used with one command `cage -- /path/to/application`. However, it can also cause issues if multiple windows are launched inside Cage, since Cage is meant to be used in kiosks, every new window will be automatically full-screened and take over the previously opened window.

To use Cage you need to:

1. Install `cage`, it should be in most repositories.
2. Run `cage -- /path/to/application` and enjoy your X11 program on niri.

Optionally one can also modify the desktop entry for the application and add the `cage --` prefix to the `Exec` property. The Spotify Flatpak for example would look something like this:

```ini
[Desktop Entry]
Type=Application
Name=Spotify
GenericName=Online music streaming service
Comment=Access all of your favorite music
Icon=com.spotify.Client
Exec=cage -- flatpak run com.spotify.Client
Terminal=false
```

## Proton-GE native Wayland

It's possible to run some games as native Wayland clients, sidestepping the issues related to X11. You can do it with a custom version of Proton like [Proton-GE](https://github.com/GloriousEggroll/proton-ge-custom) by setting the `PROTON_ENABLE_WAYLAND=1` environmental variable in the game's launch parameters. Do note that for now this is an experimental feature, might not work with every game and might have its own issues.

```
PROTON_ENABLE_WAYLAND=1 %command%
```

## Using gamescope

You can use [gamescope](https://github.com/ValveSoftware/gamescope) to run X11 games and even Steam itself.

Similar to Cage, gamescope will only show a single, topmost window, so it's not very suitable to running regular apps.
But you can run Steam in gamescope and then start some game from Steam just fine.

```
gamescope -- flatpak run com.valvesoftware.Steam
```

To run gamescope fullscreen, you can pass flags that set the necessary resolution, and a flag that starts it in fullscreen mode:

```
gamescope -W 2560 -H 1440 -w 2560 -h 1440 -f  -- flatpak run com.valvesoftware.Steam
```

> [!NOTE]
> If Steam terminates abnormally while running in gamescope, it seems that subsequent gamescope invocations will sometimes fail to start it properly.
> If this happens, run Steam inside a rootful Xwayland as described above, then exit it normally, and then you will be able to use gamescope again.

[xwayland-run]: https://gitlab.freedesktop.org/ofourdan/xwayland-run
[xwayland-satellite]: https://github.com/Supreeeme/xwayland-satellite
