When starting niri from a display manager like GDM, or otherwise through the `niri-session` binary, it runs as a systemd service.
This provides the necessary systemd integration to run programs like `mako` and services like `xdg-desktop-portal` bound to the graphical session.

Here's an example on how you might set up [`mako`](https://github.com/emersion/mako), [`waybar`](https://github.com/Alexays/Waybar), [`swaybg`](https://github.com/swaywm/swaybg) and [`swayidle`](https://github.com/swaywm/swayidle) to run as systemd services with niri.
Unlike [`spawn-at-startup`](./Configuration:-Miscellaneous.md#spawn-at-startup), this lets you easily monitor their status and output, and restart or reload them.

1. Install them, i.e. `sudo dnf install mako waybar swaybg swayidle`
2. `mako` and `waybar` provide systemd units out of the box, so you can simply add them to the niri session:

    ```
    systemctl --user add-wants niri.service mako.service
    systemctl --user add-wants niri.service waybar.service
    ```

    This will create links in `~/.config/systemd/user/niri.service.wants/`, a special systemd folder for services that need to start together with `niri.service`.

3. `swaybg` does not provide a systemd unit, since you need to pass the background image as a command-line argument.
    So we will make our own.
    Create `~/.config/systemd/user/swaybg.service` with the following contents:

    ```systemd
    [Unit]
    PartOf=graphical-session.target
    After=graphical-session.target
    Requisite=graphical-session.target

    [Service]
    ExecStart=/usr/bin/swaybg -m fill -i "%h/Pictures/LakeSide.png"
    Restart=on-failure
    ```

    Replace the image path with the one you want.
    `%h` is expanded to your home directory.

    After editing `swaybg.service`, run `systemctl --user daemon-reload` so systemd picks up the changes in the file.

    Now, add it to the niri session:

    ```
    systemctl --user add-wants niri.service swaybg.service
    ```

4. `swayidle` similarly does not provide a service, so we will also make our own.
    Create `~/.config/systemd/user/swayidle.service` with the following contents:

    ```systemd
    [Unit]
    PartOf=graphical-session.target
    After=graphical-session.target
    Requisite=graphical-session.target

    [Service]
    ExecStart=/usr/bin/swayidle -w timeout 601 'niri msg action power-off-monitors' timeout 600 'swaylock -f' before-sleep 'swaylock -f'
    Restart=on-failure
    ```

    Then, run `systemctl --user daemon-reload` and add it to the niri session:

    ```
    systemctl --user add-wants niri.service swayidle.service
    ```

That's it!
Now these three utilities will be started together with the niri session and stopped when it exits.
You can also restart them with a command like `systemctl --user restart waybar.service`, for example after editing their config files.

To remove a service from niri startup, remove its symbolic link from `~/.config/systemd/user/niri.service.wants/`.
Then, run `systemctl --user daemon-reload`.

### Running Programs Across Logout

When running niri as a session, exiting it (logging out) will kill all programs that you've started within. However, sometimes you want a program, like `tmux`, `dtach` or similar, to persist in this case. To do this, run it in a transient systemd scope:

```
systemd-run --user --scope tmux new-session
```
