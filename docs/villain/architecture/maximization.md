# Maximization separate from fullscreen

## Decision and ownership

Villain owns maximization, client protocol requests, layout and focus. Knave
owns the shared commands and window metadata. Shell presentation and its
subscription remain unchanged. No new timer, worker, subscription or persistent
configuration is introduced; the additional runtime state is one optional
window ID per workspace, one initial-focus flag per window, and bounded layout inspection.

One maximized window per workspace fills `usable_area`, preserving layer-shell
exclusive zones. Its original tiled ordering or floating rectangle stays in
place beneath the temporary geometry. Floating descendants remain visible.
Fullscreen overrides maximized geometry without discarding the saved state;
leaving fullscreen returns to maximization. Unmaximizing never exits fullscreen.
Minimizing preserves maximization until restoration, replacement by another
maximized window, or destruction. Explicitly focusing a hidden unrelated window
clears the expanded workspace states and restores the normal layout. Opening a
new unrelated application on the active workspace also clears maximization;
native clients do this once at their first commit, after parent hints are known,
and X11 clients do it at map time. New child dialogs retain parent maximization.
Background creation and ordinary later commits cannot repeat that policy, and
application fullscreen is not cleared by new-window activation. Application
requests on background workspaces do not switch workspaces. Move/resize requests
are ignored while maximized so they cannot overwrite the saved rectangle.

Windows open unmaximized. Client maximize requests before the first content
frame callback and a one-second settling period receive an unmaximized
configure; configure acknowledgements and
placeholder buffers alone do not finish opening. The existing callback delivery
path records the first frame time once per window, including captured windows.
The settling period covers toolkits that restore state after drawing, especially
XWayland; it is checked only on client requests and adds no scheduled work. Initial
X11 EWMH maximization is cleared by the normal map-time layout; requests just
after mapping also wait for the same settling period. Later client
requests and user commands can maximize. No additional timer or worker is needed.

Both native xdg-shell and XWayland EWMH requests share this implementation.
Maximized state is advertised to clients independently of fullscreen. Repeated
native requests still receive configure responses. Panel/output changes
recompute the expanded geometry; restored floating geometry remains constrained
to the current output. Maximization requests use the full usable area even when
client size hints restrict normal floating geometry, matching existing expanded
fullscreen behavior.

## Contracts, rollout and rollback

The shared contract includes `maximize-focused`, `unmaximize-focused`, and
`toggle-maximize-focused`; window summaries include default-false `maximized`.
The public flag records saved state even during fullscreen/minimization.
Existing minimize/restore commands keep their behavior. The default `MOD+F`
binding toggles maximization; custom bindings are never rewritten. Configuration
schema and binary package versions are unchanged.

Consumers: Villain, `knavectl`, and Shell's summary decoder. Update the Knave,
Villain, and Shell packages together. Rebuilding Rust consumers requires adding
the field to summary literals and accounting for command variants. There is no
persisted maximize state or migration to undo.

## Verification

Real Wayland and XWayland tests cover maximize/unmaximize, repeated requests,
fullscreen round trips, floating restoration, minimize/restore, explicit focus,
background requests and destruction. Layer-shell coverage checks the reserved
panel area and reservation changes while maximized. CLI tests cover the new
commands and argument errors; API tests cover the old summary default and wire
serialization. Nested rendering and resource measurements are recorded with the
implementation result; headless tests do not establish direct-TTY/GPU behavior.

### Local validation (2026-09-27)

All 28 Villain tests passed, including the three isolated native Wayland,
layer-shell and XWayland integration tests. Knave's 16 tests, Clippy with warnings
denied, and release builds passed. An isolated nested Winit run with two Kitty
Wayland clients verified the real `knavectl` maximize command, state snapshots,
minimize/restore, explicit focus restoration, 250 toggle requests and clean socket
shutdown. The existing callback-only probe delivered 30 callbacks over 486 ms.

A release baseline from checkout `79b0a3e` and the feature build were measured
with identical clients, cursor blinking disabled and the same callback probe.
Both had 10 threads, one XWayland child, zero additional CPU ticks and two main
thread context switches in the settled three-second idle sample. Settled RSS
was 177160 KiB baseline / 173536 KiB feature; FDs were 44 / 43 and stable. During
the toggle burst, sampled FDs were 145 before / 131 immediately after, returning
to 43 after settling. The 250 synchronous IPC requests completed in 33 ms; this
is dispatch throughput, not presentation latency. No sustained resource growth
was observed in this short run. Context switches are a wakeup proxy, not an
exhaustive count of GPU/driver wakeups. These samples do not establish long-run
or direct-TTY performance. No physical VT, monitor or installed binary was tested.

The review follow-up reproduced the hidden-new-window failure before the fix.
Extended Wayland/XWayland regressions now cover unrelated application creation,
new child dialogs, modifier-held focus, delayed background initial commits,
ordinary redraws and fullscreen preservation; all 28 tests pass. A nested run
confirmed a newly launched Kitty restores the layout and receives focus, then
passed the frame-callback probe and 250 toggles. After settling, the three-second
sample held 10 threads, one XWayland child, 43 FDs and 112344 KiB RSS, with zero
additional CPU ticks and five main-thread context switches. Clean shutdown
removed the IPC socket. Direct-TTY behavior remains unverified.
