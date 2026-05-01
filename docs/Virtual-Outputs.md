# Virtual Outputs (Headless Displays)

Niri supports creating virtual headless outputs that can be used for VNC, screen sharing, or testing purposes. Virtual outputs work similarly to Sway's `create_output` command.

## Use Cases

- Remote desktop access via VNC (e.g., with wayvnc)
- Screen recording/streaming of a separate output
- Testing and development
- Running niri without physical displays (CI, servers)

## Creating Virtual Outputs

### On a Regular Session (TTY with Physical Display)

When running niri on a TTY with your physical monitor, you can create additional virtual outputs:

```bash
# Create a 1920x1080 virtual output
niri msg create-virtual-output --width 1920 --height 1080
# Output: Created virtual output: HEADLESS-1

# Create another output with different resolution
niri msg create-virtual-output --width 1280 --height 720
# Output: Created virtual output: HEADLESS-2

# Create a 120Hz output
niri msg create-virtual-output --width 1920 --height 1080 --refresh-rate 120
# Output: Created virtual output: HEADLESS-3

# List all outputs (physical + virtual)
niri msg outputs
```

### Pure Headless Mode (No Physical Display)

For servers, VMs, or remote-only access:

```bash
# Start niri in headless mode
NIRI_BACKEND=headless niri

# A default 1920x1080 HEADLESS-1 output is created automatically
# Create additional outputs if needed
niri msg create-virtual-output --width 1280 --height 720
```

## Removing Virtual Outputs

```bash
niri msg remove-virtual-output HEADLESS-1
# Output: Removed virtual output: HEADLESS-1
```

## Configuring Virtual Outputs

Virtual outputs can be configured like regular outputs:

```bash
# Set scale
niri msg output HEADLESS-1 scale 1.5

# Set transform (rotation)
niri msg output HEADLESS-1 transform 90

# Turn off
niri msg output HEADLESS-1 off

# Turn on
niri msg output HEADLESS-1 on
```

You can also configure them in your `config.kdl`:

```kdl
output "HEADLESS-1" {
    scale 1.0
    position x=1920 y=0
}
```

## Using with VNC (wayvnc)

[wayvnc](https://github.com/any1/wayvnc) is a VNC server for wlroots-based Wayland compositors.

### Setup with Physical Display + VNC

```bash
# 1. Start niri normally on your TTY
niri

# 2. Create a virtual output for VNC
niri msg create-virtual-output --width 1920 --height 1080

# 3. Start wayvnc on the virtual output
wayvnc --output HEADLESS-1

# 4. Connect from a VNC client to your machine's IP
```

### Setup for Pure Headless (Remote Only)

```bash
# 1. Start niri in headless mode (e.g., over SSH)
NIRI_BACKEND=headless niri &

# 2. Start wayvnc
WAYLAND_DISPLAY=wayland-1 wayvnc

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

## CLI Reference

### create-virtual-output

```
niri msg create-virtual-output [OPTIONS]

Options:
  --width <WIDTH>              Width in pixels [default: 1920]
  --height <HEIGHT>            Height in pixels [default: 1080]
  --refresh-rate <REFRESH_RATE>  Refresh rate in Hz [default: 60]
```

### remove-virtual-output

```
niri msg remove-virtual-output <NAME>

Arguments:
  <NAME>  Name of the output to remove (e.g., "HEADLESS-1")
```

## Limitations

- Virtual outputs are not supported when running niri nested in another Wayland compositor (Winit backend)
- Virtual outputs are not persisted across niri restarts - you need to recreate them
