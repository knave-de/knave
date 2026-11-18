# Server-side window titlebars

## Decision

Villain honors the decoration mode requested by a Wayland client. A client
with an `xdg-decoration` object defaults to server-side mode until it requests
client-side mode. Destroying the object returns the window to client-side mode
on the next commit. Clients without that object keep their own chrome.
Managed XWayland windows get a server titlebar unless Motif hints disable it.
Fullscreen and override-redirect windows have no server titlebar.

A server titlebar reserves up to 34 logical pixels and offers minimize,
maximize/restore, and close. Overview offers the same actions plus float/tile
for each hovered window in the main workspace preview, including CSD windows.

## Ownership and rollout

Villain owns geometry, decoration negotiation, and window actions. Shell owns
the themed overview controls. The private desktop snapshot carries the preview
geometry, and Shell sends the selected action with the window ID. Deploy the
matched Knave build and restart the session. Rollback uses the prior build.
The session's GTK layout is described in [the window-control policy](0019-session-window-controls-policy.md).
