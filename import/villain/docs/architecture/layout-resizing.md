# Master/stack split resizing

Villain owns the per-workspace master ratio, divider hit testing, pointer grab,
keyboard actions and window geometry. Knave owns the optional
`[compositor] master_percent` setting (integer 10..90, default 50). The Shell
and desktop contract are unchanged. Horizontal stack dividers now resize adjacent stack pairs; see Knave ADR 0013. Villain pins knave-config at `d051735bca26affc370467779dbb1f625cac6318`.

The runtime ratio uses basis points for smooth dragging. An unset workspace
override follows configuration; a manual drag or key adjustment preserves its
value across workspace switches, maximize/minimize cycles and config reloads.
`reset-master` removes the override; restart discards all overrides. Reload
validates the complete projection before replacing it, cancels any pointer
grab, and recomputes active geometry. Background workspace layout derives the
new default when next configured. No resize writes persistent configuration.

A left press within five logical pixels of the main divider starts a Smithay
pointer grab without forwarding the press to an application or moving keyboard
focus. The resize cursor temporarily overrides the client's cursor. Geometry
uses the panel-excluding usable area. Dialogs, popups, exclusive shell surfaces,
existing grabs and expanded layouts cannot start a split drag. Window placement
runs separately from pointer refresh to avoid re-entering Smithay's pointer
lock from the grab. Workspace changes, changed tiled membership, changed usable
area, focus loss, configuration reload and button release end the drag. Escape and lifecycle cancellation restore the initial ratio. Both Winit and direct input enter the same
handler; floating-window grabs retain their existing path.

`resize-master` takes one signed integer percentage-point delta (-80..-1 or
1..80); `reset-master` takes no arguments. Defaults are MOD+Ctrl+Left/Right
(-5/+5) and MOD+Ctrl+R. Each press performs one step; custom bindings override
matching defaults. Actions are inactive without a visible master/stack split. See
README for exact TOML. These actions are compositor binding dispatches, not new
knavectl IPC commands.

## Compatibility and rollback

Existing configs retain 50/50. Publish/deploy the Knave config commit before
Villain's dependency pin. For development before publication, use a local Cargo
patch to the matching knave-config checkout. The exact git pin was verified
through Cargo's local git cache, with no committed filesystem override. No API or
binary version increment is needed for this optional setting. Rolling back Villain requires removing
new actions from custom binding sets, since old dispatch parsers reject them.
Leave the new percentage setting in place or remove it explicitly.

## Verification and resource behavior

Knave's 17 tests and Villain's 30 tests passed, including isolated Wayland,
XWayland and layer-shell tests. Coverage includes exact geometry, remainder
coverage, default parsing/writing/validation, custom bindings, real pointer
hit testing, client button isolation, unchanged keyboard focus, bounds,
workspace retention, output/workspace cancellation, expanded-layout exclusion,
reload/default/reset behavior and X11 geometry. Format, Clippy with warnings
denied and release builds passed with the pinned dependency.

An isolated nested Winit run with two Kitty clients changed reported widths
from 952/952 to 1142/762 at 60/40. It completed 100 alternating 40/60 reloads in
16 ms (dispatch throughput, not presentation latency), then cleanly removed
its socket on shutdown. The callback-only probe delivered 30 paced callbacks
in 487 ms; the old baseline delivered them in 486 ms. An initial baseline run
stalled after one callback; the comparison rerun passed on both binaries, so
that transient result is retained as a limitation rather than attributed to
this change.

No new worker, timer, subscription or persistent cache is added. State is one
optional ratio per workspace, one cursor flag and one active grab whose window
ID list is bounded by tiled membership. Pointer work is event driven; ordinary
motion outside the divider band avoids building the membership list. Changes
that do not alter the ratio do not relayout. In three-second idle samples,
baseline/feature used 1/0 CPU ticks, 125544/127464 KiB RSS, ten threads, 44 FDs
and one XWayland child. After the stress burst the feature settled at 121708 KiB,
ten threads and 42 FDs, with zero extra CPU ticks and two main-thread context
switches over three seconds. Short host-dependent samples do not establish
long-run performance; context switches are only a wakeup proxy. Protocol tests
exercise dragging, while the nested smoke verifies rendering and config-driven
resizing. Physical mouse/keyboard, direct-TTY and multi-monitor behavior were
not tested. Installed binaries were not replaced.
