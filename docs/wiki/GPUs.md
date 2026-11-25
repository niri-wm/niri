# GPUs

## Power management

<sup>Since: next release</sup>

Niri pauses idle secondary DRM devices after 10 seconds without outputs, active leases, or non-desktop connectors.
The primary rendering GPU stays active.
The kernel decides whether an idle GPU can enter runtime suspend; other clients or driver allocations can keep it awake.
Render offload clients, such as games run with `DRI_PRIME=1`, open and wake the GPU themselves.

To avoid keeping secondary GPUs awake with EGL allocations, niri uses buffers from the primary GPU for their outputs.
This can affect buffer compatibility and performance on multi-GPU systems.

Niri opens the render node in the background before adding or resuming a secondary GPU, so a slow PCIe link does not block the compositor.

For NVIDIA devices detected through the DRM sysfs driver link, niri keeps the secondary renderer and skips idle suspend when video memory preservation is disabled or its setting cannot be read.
The setting is `NVreg_PreserveVideoMemoryAllocations=1` (shown as `PreserveVideoMemoryAllocations` in `/proc/driver/nvidia/params`).

If secondary outputs fail to light up or render correctly, try the [`disable-idle-drm-device-suspend`](./Configuration:-Debug-Options.md#disable-idle-drm-device-suspend) debug flag:

```kdl
debug {
    disable-idle-drm-device-suspend
}
```

This keeps secondary GPUs active and restores their renderers and local buffer allocation.
Changing the flag while niri runs reconnects secondary outputs and ends their DRM leases.

### Checking GPU power status

Find the GPU's PCI address with `lspci -D`.
For example, for a GPU at `0000:03:00.0`:

```sh
cat /sys/bus/pci/devices/0000:03:00.0/power/runtime_status
```

A value of `suspended` means the device entered runtime suspend.
It does not establish a particular PCI power state or power consumption.
