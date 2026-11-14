# Workspace overview

`knave-shell::overview::Overview` owns the retained UI and its input state. Knave
Session starts and supervises exactly one Overview process for the Wayland
session. It remains alive while hidden and maps its exclusive full-output layer
surface only when Villain publishes `overview_visible=true`. Villain owns
workspace/window state and composes application surfaces into at most three
overview rectangles.
The shell does not configure, resize, focus, or deliver input to those preview
surfaces. Its search, card, and minimized-window controls still dispatch normal
desktop actions. Workspace navigation keeps Overview open; entering the center
card, restoring a window, and launching an app wait for acknowledgement before
hiding. Escape hides it;
Super toggles the same session-owned instance. Hiding clears the query, focused
control, browsing/pages, pointer gestures, errors, and any launch waiting on the
catalog. Reopening selects the current active workspace. The catalog, icon
cache, loader, desktop snapshot, and in-flight action bookkeeping remain owned
by the service; ordinary visible snapshot updates preserve the current search.
Disconnection also resets the view. This is private Shell behavior with no
protocol or configuration change and no added background activity.

The shell computes one selected card and up to two neighboring cards from the
current logical output size and the active workspace in each desktop snapshot.
Arrows, side cards, dots, and bar chips dispatch `FocusWorkspace` without hiding;
the subscription updates the selected card, minimized icon tray, and projected bar
together. A failed switch leaves the last confirmed workspace selected. The
existing single in-flight action bounds repeated navigation requests. No new
worker, timer, or subscription is added. It sends their interior rectangles and workspace
IDs using `set_overview_panes`. Pane contents cover the full rounded card;
minimized controls, when present, render above them. Browsing and resizing replace the entire set; search clears it.
Villain also clears it when the overview layer surface unmaps or is destroyed.
Search lists installed applications, not windows. `knave-apps` reads XDG
`.desktop` entries; entries with `NoDisplay`, `Hidden`, `Terminal=true`, or unmet
`TryExec`/`OnlyShowIn` are skipped. Launching sends the existing `Spawn { argv }`
desktop command, so Villain starts the process with the session environment and
the overview hides only after it acknowledges.

The bottom dock keeps the configured `shell.pinned_apps` in order, followed by
distinct apps with windows on the active workspace. A running pinned app appears
only once. Its icon shows a running mark and a window count when applicable;
clicking it focuses a visible window or restores a minimized one. A pin without
a window launches through the same desktop command as search. The app catalog
loads when Overview opens, off the input and frame paths. The dock pages when
its contents exceed the output width. It has no all-applications control; app
search remains the route to the full catalog.

Rows show each application's `Icon`, resolved by `knave-icons` with the icon theme
spec's size rule. Knave has no icon-theme setting and the shell reads no other
desktop's settings, so lookup tries `hicolor`, then every other installed theme by
name, then `pixmaps`.

Catalog and icon loading run on one worker owned by the `Overview`, started on
the first typed character or when a snapshot first shows minimized windows, so
their icons can resolve before any query. The input and frame paths never read
the filesystem. The job queue holds 32 jobs; icon requests are limited to
visible rows and tray pages and deduplicated, and finished icons live in a
128-entry cache. The worker wakes the host through `Application::set_waker`, and
`needs_frame` reports delivered results, so there is no polling. A scan examines
at most 16,384 directory entries and keeps at most 4,096 applications; reaching
a limit marks the catalog incomplete and logs it. Only regular files are opened,
so a FIFO cannot block the worker. The queue closes and the worker is detached
when the session ends. Enter pressed before the catalog arrives launches the top
result when it does. Minimized tray icons resolve a window's `app_id` to a
desktop entry icon; unmatched or unavailable applications reuse the generic
executable icon, and every tray entry stays a button that names its exact
window.
Icons rasterize at 64px (SVG through resvg without text or system fonts, PNG
downscaled if oversized).
Window and wallpaper textures are composed: there is no screenshot readback, PNG
transport, decoded image cache, or window input remapping in this path.

One bounded pane worker keeps IPC off the Wayland frame callback. It has a
single pending update and wakes the UI after completion, so a full queue
retries the latest layout without spawning more threads. Transient connection
or unavailable errors retry with capped backoff; permanent API errors surface
to the UI. Desktop actions use a separate bounded worker. The snapshot subscription remains event driven;
failed connections use capped retry backoff. The non-application overview host
still uses its preview worker.

This needs matching Knave, Villain, and Shell packages that support visibility
and live panes. No persistent visibility configuration changes. Direct TTY,
nested GPU composition, focus restoration, and installed binaries require
live smoke verification; a Rust build alone cannot establish them.

The `--supports-overview-service` capability probe exits successfully without
a Wayland connection. Knave Session bounds it to two seconds and checks support
before launching the service. On desktop disconnection,
Overview unmaps locally and keeps its process for reconnection. Pending
activation completions do not queue a hide for an already-hidden surface,
and the host processes requests while hidden.

The pane worker acknowledges the staged layout before the host submits its
matching UI buffer. Villain activates the layout on that surface commit, and
backdrop visibility follows mapping. Layout can be registered before the first
buffer; normal opening no longer triggers the unavailable-response backoff.
The one-slot request and acknowledgement channels coalesce work and wake the host;
no polling or additional thread is introduced. Matching Shell and Villain builds
are required; the request payload is unchanged. Rollback restores both binaries.
