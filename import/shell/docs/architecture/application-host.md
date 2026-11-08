# General shell application host

## Decision and ownership

`knave-wayland::run_application` hosts an `Application` using the existing
Wayland/WGPU runtime. The application owns scene state and typed UI actions;
the host owns connection, layer surface, seat resources, GPU lifetime and frame
callbacks. `SurfaceOptions` selects a full-output or centered logical size,
background/bottom/top/overlay layer and none/on-demand/exclusive keyboard mode.
The surface reserves no workspace area. Invalid dimensions and namespaces are
rejected before connecting.

Input is delivered in protocol order, pointer batches coalesce before drawing,
and one outstanding frame callback limits redraw scheduling. Idle applications
have no redraw timer. Desktop IPC workers are enabled only when an application
opts in through `uses_desktop`; see [the overview contract](overview.md).
Applications with background work receive a cloneable `Waker` through
`Application::set_waker` before the first frame. `wake()` uses the same one-slot
channel as the host's workers, so completed work schedules one draw without a
timer, and the application reports finished results through `needs_frame`.
Keyboard repeat uses the seat's repeat settings and an owned event-loop timer.
When both `wp_fractional_scale_v1` and `wp_viewporter` are available, the host
renders buffers at the preferred 1/120th scale, leaves `wl_surface` buffer scale
at 1, and sets the viewport destination to the configured logical surface size.
Scene, layer configure and pointer coordinates stay logical. Otherwise it uses
the compositor's integer `wl_output.scale` and buffer-scale fallback.

The first available seat owns input. Capability/seat removal cancels gestures,
releases proxies and cancels clipboard transfers. The host currently has one
surface on the compositor-selected output. Per-output hosts, native popup
surfaces and concurrent seats need a later host extension; they are not
simulated by the scene toolkit.

## Pointer cursors

The host owns a themed pointer and establishes its cursor on every pointer
entry. `Application::cursor` defaults to a visible arrow; scene applications
return `Scene::cursor()`. Standard shape requests use the compositor's cursor
shape protocol when present. Otherwise SCTK loads cursor images from the
system Xcursor theme, including its scale and hotspot. Missing shapes report an
error and fall back to the theme's arrow. The fallback uses the first theme
frame; animation of client-supplied cursor images is not implemented.

Cursor updates are coalesced until the shape, entry serial or cursor-surface
scale changes. They add no timer, thread or continuous redraw. Pointer loss
drops the pointer, shape device, cursor surface and associated theme resources.
Cursor-surface scale events are kept separate from application buffer scaling.
The existing bar/overview host also establishes arrow/hand cursors.

## Clipboard and text input

Applications return `HostRequest::Copy`/`Paste` and expose the active
`TextInputState`. Clipboard transfer is optional when the compositor provides a
data-device manager. Pipes are nonblocking, capped at four transfers and
16 KiB text, with a two-second timeout and cancellation on seat loss/shutdown.
Late paste responses carry a target ID and are rejected after focus changes.
Failures reach `Application::host_error` and stderr. After this callback the
host checks `should_close` immediately; otherwise it marks the scene dirty and
coalesces a redraw through the existing one-slot wake channel. Asynchronous
failures therefore update error UI without waiting for unrelated input.

Optional text-input-v3 publishes bounded UTF-8 surrounding text (4,000 bytes)
and the logical cursor rectangle. IME updates apply as one batch on `done`;
serial checking rejects batches for an old field. Keyboard text remains
available without the protocol. Preedit cursor/selection attributes and a real
input-method session still need dedicated verification.

## Compatibility and rollout

This is an additive workspace-internal host API at unreleased version 0.1.0;
`WaylandError::InvalidOptions` and `WaylandError::Shm` are additional exhaustive
enum variants. The cursor fallback binds the core `wl_shm` global.
Consumers of the new host rebuild with the same source revision. Existing
`run(ShellRole)` entry points, role namespaces, layer contracts and desktop IPC
payloads remain compatible. No settings or persistent state are migrated.
Villain retains window/workspace/focus policy; this change modifies no
compositor code. Revert the host changes to remove the new entry point while
retaining the previous shell role behavior.

## Verification and remaining coverage

Workspace tests, Clippy and release builds cover all consuming crates. GPU
readback is separate from live Wayland smoke coverage. A sized generic surface
was launched on the development Hyprland session and stayed alive; over a
five-second idle sample it held 3 threads, 67 FDs and about 212 MiB RSS with no
CPU-tick or context-switch increase. This is a short baseline, not a long-run
leak test. Clipboard interoperability, real IME composition, output hotplug,
installed binaries and direct-TTY behavior are not established by that sample.

Cursor regressions cover entry/leave, shape changes, capture, overrides, disabled
controls and 10,000 repeated motions without duplicate requests. A live
Hyprland trace confirmed pointer entry followed by native arrow/hand requests.
The cursor-image fallback and scaled-output appearance have not been live
verified. The brief run settled at three threads and 67 FDs, matching the prior
host baseline; this does not establish long-run resource behavior.

## Static wallpaper consumer

`knave-shell wallpaper ABSOLUTE_PNG_PATH cover|contain|stretch` is supervised by
Knave Session when `[shell.wallpaper].image` is set. It uses the background layer
and no keyboard interactivity, loads one bounded PNG and keeps black behind it.
Decode errors are logged and fall back to black. There is no watcher or redraw
timer. Configuration changes apply on session restart; the canonical settings
and lifecycle decision are documented in Knave ADR 0008.

Overview uses a transparent clear color; Villain places a 50% black dimmer behind
the full-card previews and shell controls. The host selects premultiplied surface alpha when supported, matching
its blended render output. Villain includes the existing wallpaper texture in
workspace panes with centered cover scaling and rounded masks, and suppresses
full-size desktop windows and the bar behind Overview. Empty workspaces omit the
minimized icon tray; when needed, its controls render above the wallpaper.
