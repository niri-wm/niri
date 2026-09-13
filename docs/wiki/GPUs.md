This page covers NVIDIA quirks and multi-GPU setups, including NVIDIA Optimus and eGPU docks.

## NVIDIA

### High VRAM usage fix

Presently, there is a quirk in the NVIDIA drivers that affects niri's VRAM usage (the driver does not properly release VRAM back into the pool). Niri *should* use on the order of 100 MiB of VRAM (as checked in [nvtop](https://github.com/Syllo/nvtop)); if you see anywhere close to 1 GiB of VRAM in use, you are likely hitting this issue (heap not returning freed buffers to the driver).

Luckily, you can mitigate this by configuring the NVIDIA drivers with a per-process application profile as follows:

* `sudo mkdir -p /etc/nvidia/nvidia-application-profiles-rc.d` to make the config dir if it does not exist (it most likely does not if you are reading this)
* write the following JSON blob to set the `GLVidHeapReuseRatio` config value for the `niri` process into the file `/etc/nvidia/nvidia-application-profiles-rc.d/50-limit-free-buffer-pool-in-wayland-compositors.json`:

    ```json
    {
        "rules": [
            {
                "pattern": {
                    "feature": "procname",
                    "matches": "niri"
                },
                "profile": "Limit Free Buffer Pool On Wayland Compositors"
            }
        ],
        "profiles": [
            {
                "name": "Limit Free Buffer Pool On Wayland Compositors",
                "settings": [
                    {
                        "key": "GLVidHeapReuseRatio",
                        "value": 0
                    }
                ]
            }
        ]
    }
    ```

    (The file in `/etc/nvidia/nvidia-application-profiles-rc.d/` can be named anything, and does not actually need an extension).

Restart niri after writing the config file to apply the change.

The upstream issue that this solution was pulled from is [here](https://github.com/NVIDIA/egl-wayland/issues/126#issuecomment-2379945259). There is a (slim) chance that NVIDIA updates their built-in application profiles to apply this to niri automatically; it is unlikely that the underlying heuristic will see a proper fix.

The fix shipped in the driver at the time of writing uses a value of 0, while the initial config posted by an Nvidia engineer approximately a year prior used a value of 1.

### Screencast flickering fix

<sup>Until: 25.08</sup>

If you have screencast glitches or flickering on NVIDIA, set this in the niri config:

```kdl,must-fail
debug {
    wait-for-frame-completion-in-pipewire
}
```

This debug flag has since been removed because the problem was properly fixed in niri.

## Secondary GPUs (Optimus, eGPU)

### Power management

When nothing is using a secondary GPU (dGPU, eGPU) for outputs or leases, niri suspends it. Saves power, and on some systems the device drops into ACPI `D3cold`.

On hotplug, niri wakes the GPU in the background (up to 10 seconds), so slow PCIe links don't stall the compositor.

If suspend gives you trouble, turn it off:

```kdl
debug {
    disable-idle-drm-device-suspend
}
```

On NVIDIA, suspend is skipped unless video memory preservation is on — without it, suspending can kill the EGL context.

### eGPU connection latency

Some eGPUs are slow to come back after hotplug. Niri opens the render node from a background thread, so a slow PCIe resume doesn't block the compositor.
