# Server-side window titlebars

## Decision

Villain draws a separate titlebar above every managed Wayland toplevel and managed X11 window. It owns the frame geometry, controls, and pointer handling. Fullscreen windows and X11 override-redirect surfaces remain undecorated. The titlebar adds 34 logical pixels to the managed frame; short windows clamp the reserved height to keep client geometry positive.

Wayland clients receive server-side mode through `xdg-decoration` regardless of their requested mode. The compositor answers client-side mode requests with server-side mode. Destroying a decoration object switches the toplevel to client-side mode on the next commit, as required by the protocol; a toplevel that never created the object still gets a compositor frame. X11 Motif decoration hints do not disable the managed frame. Applications that draw custom controls inside their client content can still show those controls alongside the server titlebar; Knave does not crop or infer arbitrary application chrome.

## Ownership and rollout

Villain owns rendering, hit testing, geometry, and decoration negotiation. Knave Session supplies the locked GTK `button-layout=":"` preference to owned application launches, without modifying the user's persistent preferences. There is no configuration key, migration, schema, or wire contract change. Rollback is the prior coordinated Knave build and a new session.

Verification covers Wayland protocol negotiation and geometry, managed X11 geometry, and the isolated desktop harness. The GTK setting is not a universal Electron control; applications with custom frameless windows require application-specific validation.
