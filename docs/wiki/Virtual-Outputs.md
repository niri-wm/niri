# Virtual Outputs

Virtual outputs are extra `wl_output`s (usually named `HEADLESS-*`) that niri can create even when
there’s no physical monitor attached (or in addition to real monitors).

They’re useful for:

- Sunshine / Moonlight (wlr-screencopy capture)
- VNC (e.g. wayvnc)
- “headless” remote sessions and general screen sharing

## Creating virtual outputs

### TTY backend (regular session with physical displays)

When running niri on a TTY with your physical monitor, you can create additional virtual outputs:

```bash
# Create a 1920x1080@144 output
niri msg create-virtual-output --width 1920 --height 1080 --refresh-rate 144
# Output: Created virtual output: HEADLESS-1
```

If you want to see what exists:

```bash
niri msg outputs
```

### Headless backend (no physical displays)

For servers or remote-only access (e.g. SSH) where a real TTY is not available:

```bash
# Start niri in headless mode
NIRI_BACKEND=headless niri --session &
# A 1920x1080@60 HEADLESS-1 virtual output is created by default
```

Note: if you’re running commands from another shell, you may need to set `WAYLAND_DISPLAY` to the
socket of that headless session.

## Removing virtual outputs

```bash
niri msg remove-virtual-output HEADLESS-1
# Output: Removed virtual output: HEADLESS-1
```

## Configuring virtual outputs

Virtual outputs can be configured like regular outputs:

```bash
# Enable/disable
niri msg output HEADLESS-1 <on|off>

# Set scale
niri msg output HEADLESS-1 scale 1.25

# Set transform
niri msg output HEADLESS-1 transform 90
```

You can also configure them in your `config.kdl` file:

```kdl
output "HEADLESS-1" {
    scale 1.25
    transform "90"
    position x=1920 y=0
}
```

## Using with Sunshine

If you want, you can tell [Sunshine](https://github.com/LizardByte/Sunshine) to create a virtual output that matches the client ([Moonlight](https://github.com/moonlight-stream/moonlight-qt)) resolution + refresh rate when a session starts, and remove it when the session ends.
resolution + refresh rate when a session starts, and remove it when the session ends.

The snippets below are intentionally minimal; adjust them to your Sunshine setup.

### TTY backend (create + remove per session)

```json
{
  "apps": [
    {
      "name": "Remote Desktop",
      "output": "HEADLESS-1",
      "prep-cmd": [
        {
          "do": "sh -c \"niri msg create-virtual-output --width ${SUNSHINE_CLIENT_WIDTH} --height ${SUNSHINE_CLIENT_HEIGHT} --refresh-rate ${SUNSHINE_CLIENT_FPS}\"",
          "undo": "niri msg remove-virtual-output HEADLESS-1"
        }
      ]
    }
  ]
}
```

### Headless backend (reuse the default output)

In headless mode, niri creates `HEADLESS-1` by default. You can just change its mode for the
session and restore it afterwards.

```json
{
  "apps": [
    {
      "name": "Remote Desktop",
      "output": "HEADLESS-1",
      "prep-cmd": [
        {
          "do": "sh -c \"niri msg output HEADLESS-1 custom-mode \\\"${SUNSHINE_CLIENT_WIDTH}x${SUNSHINE_CLIENT_HEIGHT}@${SUNSHINE_CLIENT_FPS}\\\"\"",
          "undo": "niri msg output HEADLESS-1 mode '1920x1080@60.000'"
        }
      ]
    }
  ]
}
```

## Input and seats (headless mode)

In headless mode, niri can still use libinput to read local input devices (if it has permission to
open `/dev/input/event*`). This is independent of virtual outputs.

libinput enumerates devices by udev seat. By default niri uses `seat0`, but you can override it:

```bash
XDG_SEAT=seat0 NIRI_BACKEND=headless niri --session
```

The Wayland `wl_seat` name exposed by niri matches this seat string (`XDG_SEAT` / `seat0`).
If niri can’t access any input devices (permissions, container, etc.), it will still start; you’ll
just have no _local_ input.

This section is mostly about _local_ kernel input devices. If you’re using a remote client that
injects input over Wayland (like wayvnc), niri doesn’t need access to `/dev/input` for that.

## Using with wayvnc

[wayvnc](https://github.com/any1/wayvnc) is a VNC server for wlroots-based Wayland compositors. It forwards keyboard and pointer input from VNC clients to the compositor using Wayland “virtual input” protocols (virtual keyboard + virtual pointer). This means you can have working remote input even if the headless niri process can’t open `/dev/input/event*`.

### Physical displays + extra virtual output

```bash
# 1. Start niri normally on your TTY
niri --session &

# 2. Create a virtual output for VNC
niri msg create-virtual-output --width 1920 --height 1080

# 3. Start wayvnc on the virtual output
wayvnc --output HEADLESS-1

# 4. Connect from a VNC client to your machine's IP
```

### Pure headless (remote only)

```bash
# 1. Start niri in headless mode (e.g., over SSH)
NIRI_BACKEND=headless niri &

# 2. Start wayvnc
WAYLAND_DISPLAY=wayland-1 wayvnc --output HEADLESS-1

# 3. Connect from a VNC client
```

### Headless with systemd

For a persistent headless niri session:

```ini
# ~/.config/systemd/user/niri-headless.service
[Unit]
Description=Niri Headless Session

[Service]
Type=simple
Environment=NIRI_BACKEND=headless
ExecStart=/usr/bin/niri
Restart=on-failure

[Install]
WantedBy=default.target
```

```bash
systemctl --user enable --now niri-headless
```
