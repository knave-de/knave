/* Protocol regression: frame-only commits must keep receiving paced callbacks. */
#define _GNU_SOURCE
#include <assert.h>
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>
#include <wayland-client.h>
#include "xdg-shell-client-protocol.h"

static struct wl_compositor *compositor;
static struct wl_shm *shm;
static struct xdg_wm_base *wm;
static struct wl_surface *surface;
static struct wl_buffer *buffer;
static int configured, frames, closed;
static uint32_t first_time, last_time;
static void request_frame(void);
static void frame_done(void *data, struct wl_callback *callback, uint32_t time) {
    (void)data;
    wl_callback_destroy(callback);
    if (frames == 5) first_time = time;
    last_time = time;
    frames++;
    if (frames < 35) request_frame();
}
static const struct wl_callback_listener callback_listener = { frame_done };
static void request_frame(void) {
    struct wl_callback *callback = wl_surface_frame(surface);
    wl_callback_add_listener(callback, &callback_listener, NULL);
    /* Warm the damage tracker's buffer history, then commit callbacks only. */
    if (frames < 5) {
        wl_surface_attach(surface, buffer, 0, 0);
        wl_surface_damage_buffer(surface, 0, 0, 64, 64);
    }
    wl_surface_commit(surface);
}
static void ping(void *data, struct xdg_wm_base *base, uint32_t serial) {
    (void)data;
    xdg_wm_base_pong(base, serial);
}
static const struct xdg_wm_base_listener wm_listener = { ping };
static void configure(void *data, struct xdg_surface *xdg, uint32_t serial) {
    (void)data;
    xdg_surface_ack_configure(xdg, serial);
    if (configured) return;
    configured = 1;
    wl_surface_attach(surface, buffer, 0, 0);
    request_frame();
}
static const struct xdg_surface_listener surface_listener = { configure };
static void top_configure(void *data, struct xdg_toplevel *top, int32_t w, int32_t h, struct wl_array *states) {
    (void)data; (void)top; (void)w; (void)h; (void)states;
}
static void top_close(void *data, struct xdg_toplevel *top) {
    (void)data; (void)top; closed = 1;
}
static const struct xdg_toplevel_listener top_listener = { .configure = top_configure, .close = top_close };
static void global(void *data, struct wl_registry *registry, uint32_t name, const char *interface, uint32_t version) {
    (void)data; (void)version;
    if (!strcmp(interface, "wl_compositor")) compositor = wl_registry_bind(registry, name, &wl_compositor_interface, 4);
    if (!strcmp(interface, "wl_shm")) shm = wl_registry_bind(registry, name, &wl_shm_interface, 1);
    if (!strcmp(interface, "xdg_wm_base")) wm = wl_registry_bind(registry, name, &xdg_wm_base_interface, 1);
}
static void global_remove(void *data, struct wl_registry *registry, uint32_t name) {
    (void)data; (void)registry; (void)name;
}
static const struct wl_registry_listener registry_listener = { global, global_remove };
static int64_t milliseconds(void) {
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    return now.tv_sec * 1000 + now.tv_nsec / 1000000;
}
int main(void) {
    struct wl_display *display = wl_display_connect(NULL);
    assert(display);
    struct wl_registry *registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, NULL);
    assert(wl_display_roundtrip(display) >= 0);
    assert(compositor && shm && wm);
    xdg_wm_base_add_listener(wm, &wm_listener, NULL);
    int fd = memfd_create("villain-frame-test", MFD_CLOEXEC);
    assert(fd >= 0 && ftruncate(fd, 64 * 64 * 4) == 0);
    uint32_t *pixels = mmap(NULL, 64 * 64 * 4, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    assert(pixels != MAP_FAILED);
    for (int i = 0; i < 64 * 64; ++i) pixels[i] = 0xff365080;
    struct wl_shm_pool *pool = wl_shm_create_pool(shm, fd, 64 * 64 * 4);
    buffer = wl_shm_pool_create_buffer(pool, 0, 64, 64, 64 * 4, WL_SHM_FORMAT_XRGB8888);
    wl_shm_pool_destroy(pool);
    close(fd);
    surface = wl_compositor_create_surface(compositor);
    struct xdg_surface *xdg = xdg_wm_base_get_xdg_surface(wm, surface);
    xdg_surface_add_listener(xdg, &surface_listener, NULL);
    struct xdg_toplevel *top = xdg_surface_get_toplevel(xdg);
    xdg_toplevel_add_listener(top, &top_listener, NULL);
    xdg_toplevel_set_title(top, "Villain frame callback regression");
    xdg_toplevel_set_app_id(top, "villain-frame-test");
    wl_surface_commit(surface);
    int64_t deadline = milliseconds() + 3000;
    while (frames < 35 && !closed && milliseconds() < deadline) {
        assert(wl_display_dispatch_pending(display) >= 0);
        assert(wl_display_flush(display) >= 0);
        struct pollfd pfd = { .fd = wl_display_get_fd(display), .events = POLLIN };
        if (poll(&pfd, 1, 50) > 0) {
            if (wl_display_dispatch(display) < 0) break;
        }
    }
    uint32_t elapsed = frames > 5 ? last_time - first_time : 0;
    printf("frames=%d frame_only=%d callback_span_ms=%u\n", frames, frames > 5 ? frames - 5 : 0, elapsed);
    xdg_toplevel_destroy(top);
    xdg_surface_destroy(xdg);
    wl_surface_destroy(surface);
    wl_buffer_destroy(buffer);
    wl_display_flush(display);
    wl_display_disconnect(display);
    munmap(pixels, 64 * 64 * 4);
    /* Detect both starvation and an unpaced callback busy loop. */
    return frames == 35 && elapsed >= 50 && elapsed < 2000 ? 0 : 1;
}
