#define _POSIX_C_SOURCE 200809L
#include <errno.h>
#include <fcntl.h>
#include <gbm.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
#include <wayland-client.h>
#include "linux-dmabuf-client.h"

static struct zwp_linux_dmabuf_v1 *dmabuf;
static int result;
static int keep_buffers;
static int kept_count;
static struct wl_buffer *kept[1024];

static void global(void *data, struct wl_registry *registry, uint32_t id,
                   const char *interface, uint32_t version) {
    (void)data;
    if (strcmp(interface, "zwp_linux_dmabuf_v1") == 0)
        dmabuf = wl_registry_bind(registry, id, &zwp_linux_dmabuf_v1_interface,
                                  version < 3 ? version : 3);
}
static void global_remove(void *data, struct wl_registry *registry, uint32_t id) {
    (void)data; (void)registry; (void)id;
}
static const struct wl_registry_listener registry_listener = {global, global_remove};
static void format(void *data, struct zwp_linux_dmabuf_v1 *obj, uint32_t f) {
    (void)data; (void)obj; (void)f;
}
static void modifier(void *data, struct zwp_linux_dmabuf_v1 *obj, uint32_t f,
                     uint32_t hi, uint32_t lo) {
    (void)data; (void)obj; (void)f; (void)hi; (void)lo;
}
static const struct zwp_linux_dmabuf_v1_listener dmabuf_listener = {format, modifier};
static void created(void *data, struct zwp_linux_buffer_params_v1 *params,
                    struct wl_buffer *buffer) {
    (void)data; (void)params;
    /* Never attach: no presentation or frame callback can clean this import. */
    if (keep_buffers)
        kept[kept_count++] = buffer;
    else
        wl_buffer_destroy(buffer);
    result = 1;
}
static void failed(void *data, struct zwp_linux_buffer_params_v1 *params) {
    (void)data; (void)params;
    result = -1;
}
static const struct zwp_linux_buffer_params_v1_listener params_listener = {created, failed};

int main(int argc, char **argv) {
    if (argc != 4 && argc != 5) {
        fprintf(stderr, "usage: %s RENDER_NODE COUNT INTERVAL_MS [HOLD_SECONDS]\n", argv[0]);
        return 2;
    }
    int count = atoi(argv[2]), interval = atoi(argv[3]);
    int hold_seconds = argc == 5 ? atoi(argv[4]) : 0;
    if (count < 1 || count > 1024 || interval < 10 || interval > 1000) return 2;
    if (hold_seconds < 0 || hold_seconds > 60) return 2;
    keep_buffers = hold_seconds > 0;
    struct wl_display *display = wl_display_connect(NULL);
    if (!display) { perror("wl_display_connect"); return 1; }
    struct wl_registry *registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, NULL);
    if (wl_display_roundtrip(display) < 0 || !dmabuf) {
        fprintf(stderr, "linux-dmabuf unavailable\n"); return 1;
    }
    zwp_linux_dmabuf_v1_add_listener(dmabuf, &dmabuf_listener, NULL);
    if (wl_display_roundtrip(display) < 0) return 1;
    int device_fd = open(argv[1], O_RDWR | O_CLOEXEC);
    if (device_fd < 0) { perror("open render node"); return 1; }
    struct gbm_device *device = gbm_create_device(device_fd);
    if (!device) { fprintf(stderr, "gbm_create_device failed\n"); return 1; }
    const uint32_t width = 1024, height = 1024;
    struct timespec delay = {interval / 1000, (interval % 1000) * 1000000L};
    for (int i = 0; i < count; ++i) {
        struct gbm_bo *bo = gbm_bo_create(device, width, height, GBM_FORMAT_XRGB8888,
                                         GBM_BO_USE_RENDERING | GBM_BO_USE_LINEAR);
        if (!bo) { perror("gbm_bo_create"); return 1; }
        int fd = gbm_bo_get_fd(bo);
        if (fd < 0) { perror("gbm_bo_get_fd"); return 1; }
        uint64_t mod = gbm_bo_get_modifier(bo);
        struct zwp_linux_buffer_params_v1 *params = zwp_linux_dmabuf_v1_create_params(dmabuf);
        zwp_linux_buffer_params_v1_add_listener(params, &params_listener, NULL);
        zwp_linux_buffer_params_v1_add(params, fd, 0, 0, gbm_bo_get_stride(bo), mod >> 32, mod);
        result = 0;
        zwp_linux_buffer_params_v1_create(params, width, height, GBM_FORMAT_XRGB8888, 0);
        close(fd);
        while (!result && wl_display_dispatch(display) >= 0) {}
        zwp_linux_buffer_params_v1_destroy(params);
        gbm_bo_destroy(bo);
        if (result != 1 || wl_display_roundtrip(display) < 0) {
            fprintf(stderr, "import failed at %d\n", i); return 1;
        }
        nanosleep(&delay, NULL);
    }
    if (keep_buffers) {
        printf("holding %d live buffers (%u MiB total)\n", kept_count, kept_count * 4);
        fflush(stdout);
        struct timespec hold = {hold_seconds, 0};
        while (nanosleep(&hold, &hold) < 0 && errno == EINTR) {}
        for (int i = 0; i < kept_count; ++i)
            wl_buffer_destroy(kept[i]);
        if (wl_display_roundtrip(display) < 0) return 1;
    }
    printf("imported and destroyed %d buffers (%u MiB total)\n", count, count * 4);
    gbm_device_destroy(device);
    close(device_fd);
    zwp_linux_dmabuf_v1_destroy(dmabuf);
    wl_registry_destroy(registry);
    wl_display_disconnect(display);
    return 0;
}
