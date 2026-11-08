# Desktop state subscriptions

The desktop host consumes snapshots via one dedicated `DesktopSubscription`.
The first frame initializes state; every later frame replaces it. There is no
healthy-connection timer or query loop. The existing one-entry UI mailbox and
Wayland wake channel coalesce updates. Commands and preview requests use their
own connections. Subscription events do not wait for a preview response.

Close interrupts the socket read and joins the worker. Failed connections report
an explicit error and reconnect with 250 ms to 5 s backoff; idle connections have
no receive timeout. Reconnect always processes a new initial snapshot and
invalidates previews, including when the new compositor reuses generation IDs.
A connection in its initial handshake can take up to the existing two-second
transport timeout to cancel. The old 500 ms polling fallback is removed.

Knave, Villain, and Shell must use the same desktop request and snapshot shapes.
Update the three packages together. Disconnections report an explicit error,
not fabricated empty state or a polling fallback. Existing Application callbacks
and user configuration are unchanged. No frame pacing or pixel-damage streaming
is added.

## Measurement

A live Wayland host probe on 2026-09-27 received pushed snapshots, survived a
connection drop and a reconnect with the same generation, restored a minimized
window through the separate command connection, and exited cleanly while the
subscription was idle. The service observed two subscription handshakes and
zero snapshot queries, including a three-second idle interval. This exercises
the host transport and lifecycle with a controlled service; it does not establish
direct-TTY frame latency or pixel-preview freshness. Workspace transition IPC
latency and subscriber resource measurements are recorded in Villain's companion
architecture note. No unit-test suite was run for this change.
