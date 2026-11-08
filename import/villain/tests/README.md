# Wayland frame callback regression

Run `tests/run-frame-callbacks.sh` against a test compositor selected by
`WAYLAND_DISPLAY` and `XDG_RUNTIME_DIR`. It needs a C compiler, `pkg-config`,
`wayland-scanner`, and the Wayland client/protocol development packages.

The client opens a temporary window, warms up rendering, then requests 30 frame
callbacks without changing pixels or attaching another buffer. It fails if the
callbacks stall or arrive in an unpaced busy loop. The window closes automatically
within three seconds. Run it in an otherwise idle compositor, with the pointer
still: unrelated damage can hide the original failure.

Both Villain backends use the same callback scheduler. For the nested backend,
start `villain --winit` with a private runtime directory and point the test at
its socket. For a direct DRM test, run it in the active Villain TTY session.

The native Wayland window-request regression and XWayland input regression use
private headless compositors. They exercise fullscreen transitions, restoration,
fixed-size hints, parent dialogs, background requests, and pointer move/resize
grabs. XWayland must be installed for its test. The layer-shell regression checks the initial configure handshake, buffer mapping,
reserved panel space, fullscreen geometry, focus modes, workspace independence,
popups outside parent bounds, popup grabs, unmapping, remapping, and destruction.
Run all three with:

```sh
test_runtime=$(mktemp -d)
XDG_RUNTIME_DIR="$test_runtime" XDG_CONFIG_HOME="$test_runtime" \
  cargo test -p villain -- --ignored --nocapture --test-threads=1
```
