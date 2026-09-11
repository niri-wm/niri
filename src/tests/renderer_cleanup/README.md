# linux-dmabuf cleanup reproducer

`dmabuf-churn.c` imports distinct 1024x1024 GBM buffers into niri and destroys
their Wayland buffers. It creates no surfaces and requests no frame callbacks.
This exercises the cached EGLImage path in addition to the automated EGL
tests' deferred texture destruction path.

Build with Wayland, wayland-protocols, wayland-scanner, GBM, and a C compiler:

```sh
protocols=$(pkg-config --variable=pkgdatadir wayland-protocols)
protocol=$protocols/stable/linux-dmabuf/linux-dmabuf-v1.xml
if [ ! -f "$protocol" ]; then
    protocol=$protocols/unstable/linux-dmabuf/linux-dmabuf-unstable-v1.xml
fi
wayland-scanner client-header "$protocol" /tmp/linux-dmabuf-client.h
wayland-scanner private-code "$protocol" /tmp/linux-dmabuf-protocol.c
cc -Wall -Wextra -O2 -I/tmp src/tests/renderer_cleanup/dmabuf-churn.c \
    /tmp/linux-dmabuf-protocol.c $(pkg-config --cflags --libs wayland-client gbm) \
    -o /tmp/dmabuf-churn
```

Use a disposable niri instance with the matching `WAYLAND_DISPLAY` and
`NIRI_SOCKET`. Select its GPU's render node below. The example imports
512 MiB cumulatively and temporarily powers off that instance's outputs:

```sh
niri msg action power-off-monitors
/tmp/dmabuf-churn /dev/dri/renderD128 128 100
sleep 4
# Inspect niri's GPU allocation here, before turning outputs back on.
niri msg action power-on-monitors
```

With the fix, the destroyed imports are reclaimed while the outputs remain
off. Without it, the tested AMDGPU instance retained the 512 MiB until it
rendered again. Process RSS does not account for these GPU allocations; use
DRM fdinfo or GPU tooling. Do not sum duplicate fdinfo entries with the same
DRM device/client ID.

An optional fourth argument keeps all imported buffers alive for that many
seconds before destroying them. For example, `32 10 6` should retain 128 MiB
for six seconds, then reclaim it without requiring a frame. The automated
`egl_cleanup_*` tests also check that live textures survive maintenance and
that subsequent ticks reclaim resources dropped later.
