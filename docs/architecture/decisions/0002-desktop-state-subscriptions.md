# Desktop state subscriptions (API 1.2)

The shell's 500 ms snapshot poll delays state updates and wakes idle desktops.
Knave owns an additive `subscribe` request carrying `protocol: {major, minor}`.
Villain registers it on the compositor event loop and replies with a complete
`DesktopResponse::Snapshot`, then streams replacement snapshots on the same
connection. Initial registration and capture are atomic with respect to desktop
mutations. There are no heartbeats, deltas, replay log, or subscription IDs.

The stream is state synchronization, not an audit log. Slow readers may skip
intermediate states; every delivered frame is complete and ordered. Generation
is a compositor-session-local state revision, not a window allocation counter.
Reconnect always starts with a new initial snapshot, irrespective of generation.
Close the socket to unsubscribe; use a separate connection for commands/queries.

Villain coalesces relevant mutations after event dispatch. Creation, destruction,
workspace/focus changes, minimization, fullscreen, maximization, and title/app-ID
changes invalidate state. Pixel-only commits and cursor motion do not broadcast.
This does not provide live preview damage notifications or change frame pacing.

API 1.0/1.1 query/action clients retain their wire behavior with a 1.2 server.
Subscription requires matching major 1 and minor >=2 supported by the server.
Older servers reject the new request. The shell reports that error and retries
with bounded reconnect backoff; it does not fall back to polling. Source clients
matching DesktopRequest exhaustively must handle Subscribe. Binary/library
versions remain unreleased 0.1.0; no configuration schema or socket-path change.

Resources: at most 16 client workers across queries/actions/subscriptions;
nonblocking listener, one latest snapshot slot per subscriber, shared serialized
snapshot bytes bounded to 16 MiB, and a wake socket pair per connection. Socket
writes and request handoff have bounded timeouts. Slow/disconnected readers are
removed without blocking the compositor; server shutdown closes sockets and
joins workers. The shell blocks on its dedicated stream while healthy, interrupts
it on close, and wakes for backoff only while disconnected.

Rollout: publish the Knave API revision, deploy matching Villain, then deploy the
shell. Roll back the shell first. Existing clients do not need migration.
