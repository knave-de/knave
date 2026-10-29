# Optional empty-workspace skipping

The original wrapping behavior in this decision is superseded by
[ADR 0016](0016-dynamic-workspaces.md), which keeps navigation linear and grows
the workspace list at the forward edge.

Status: accepted for the coordinated workspace-skip-empty change.

Knave owns the additive typed `[compositor.workspace] skip_empty` boolean and
preservation-aware writes. Villain owns the bounded destination search, input
policy and rendering. The setting defaults to false, is reloadable, and needs
no migration, config version, desktop API change or second persistence store.
Consumers are knave-config loader/writer, settings and session normalization,
and Villain's runtime loader/reloader; Shell and portal are rebuilt together.
Deploy the Knave contract and update Villain's published dependency pin together.

When enabled, next/previous navigation searches the other workspace slots in
numeric order with wraparound for one containing any managed window, including
minimized windows. Starting on an empty workspace uses the same directional
search. Without another eligible workspace, navigation is a no-op. Explicit
number/bar/IPC selection still reaches any slot. Closing a workspace's last
window does not automatically switch focus.

Shortcuts and gesture previews share the same target resolver. A swipe locks its
previewed target until travel reverses or ends, and commits that target through
the existing FocusWorkspace dispatch. If its target becomes empty before
release, cancel rather than retarget. Reload and the existing gesture abort
paths keep their ownership/cancellation behavior. Rendering stores travel
direction explicitly, so a jump from workspace 1 to 4 still slides left.

The search examines at most the fixed workspace count with a stack occupancy
array. No thread, watcher, cache or timer is added. Runtime checks compare idle,
normal and stress resource samples. Verify default/off and opt-in behavior,
sparse/wrapped navigation, empty and minimized-only workspaces, direct selection,
preview/commit consistency, reversal and target destruction. Run the coordinated
build, frozen desktop harness and recorded feature assertions. Synthetic nested
input/software rendering does not prove physical touchpad or direct DRM/GPU
behavior. Rollback restores the previous coordinated binaries; remove the key or
set it false to restore sequential navigation on the current binaries.
