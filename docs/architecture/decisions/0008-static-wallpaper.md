# Static wallpaper

Knave owns the optional `[shell.wallpaper]` settings. `image` is an
optional absolute PNG path; omission keeps the black compositor background.
`fit` is `cover` (default, centered crop), `contain` (whole image with black
letterboxing), or `stretch` (fills output without preserving aspect ratio).
Atomic settings writes preserve unknown
wallpaper keys and remove `image` when cleared.

Session passes the validated path and fit as separate arguments to one supervised
`knave-shell wallpaper PATH FIT` process. Shell owns decoding and presentation
using its general application host on the background layer, without keyboard
focus or exclusive workspace area. Deploy matching Session and Shell builds.
The desktop IPC and Villain configuration contracts remain unchanged.

PNG input is capped at 64 MiB, 16384 pixels per side and 16 Mi pixels total;
decoder storage is capped at 128 MiB. Missing/invalid images log an error and
render black. One immutable decoded image and its GPU texture live for the
process lifetime and are released on exit. There are no image watchers, worker
threads or redraw timers. The host draws on configure/expose/input events.
Changes apply on session restart; `knavectl reload` remains compositor-only.
The host currently targets one output. Workspace-only captures exclude shell layers, including wallpaper. Overview
workspace panes and PNG previews include the wallpaper background surface but
exclude other shell layers. Full-output captures include the wallpaper. Per-output/workspace selection and live reload are deferred.

Rollback uses the previous Session/Shell pair; the unknown wallpaper table can
remain in configuration. Verification uses config round trips, decoder tests,
workspace checks and the isolated software-rendered desktop harness, including
workspace switching, screenshot inspection and idle/stress resource samples.
Direct-TTY and hardware-GPU behavior require separate verification.

## Software-rendered verification (2026-10-16)

The selected 1470x981 image was converted from AVIF to PNG outside the runtime.
Rendered cover-mode pixels matched the scaled source on workspaces 1 and 2
(mean absolute channel error 0.18/255 over 80,000 pixels). A command typed into
Kitty produced its expected output file. Twenty workspace switches left wallpaper
RSS (230456 KiB), 12 threads and 47 FDs unchanged. Two settled three-second idle
samples had zero CPU-tick or main-thread context-switch growth. This initial
separate WGPU process costs about 225 MiB RSS on llvmpipe. These short samples do
not establish long-run or hardware-GPU performance. The recorded test session
remains available over VNC.
