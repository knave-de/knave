# Villain component boundaries

Villain owns compositor and window-manager behavior:

- Wayland compositor operation and surface lifecycle;
- layout, workspaces, focus, and window management;
- input policy and compositor-side interaction; and
- the private compositor protocol used by Knave components.

Knave owns session lifecycle, user-facing settings, the public desktop API, and
shell/UI presentation. Villain must consume explicit projections or protocol
messages rather than reading Knave's private state or writing the user's
canonical configuration.

Villain's shutdown path must release its owned Wayland socket and lock on
SIGINT, SIGTERM, initialization failure, and normal event-loop exit. Socket
cleanup is type-checked and scoped to the selected runtime display name.
Changes involving focus, input, surface commits, layer-shell, XWayland, VT
selection, or process shutdown must be reviewed as behavior changes, not only
as local refactors.
