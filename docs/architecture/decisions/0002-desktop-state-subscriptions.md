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
transmitted snapshot frames capped at 16 MiB, and a wake socket pair per
connection. Serialization can allocate more transiently before the server checks
the frame size. The client stops reading and closes the connection on an oversized
frame, so its unread tail cannot be interpreted as another snapshot. Pending
request handoff has a two-second timeout; a request already executing waits for
its result. Socket writes also time out. Slow/disconnected readers are removed
without blocking the compositor; server shutdown closes sockets and joins
workers. The shell blocks on its dedicated stream while healthy, interrupts it
on close, and wakes for backoff only while disconnected.

Rollout: publish the Knave API revision, deploy matching Villain, then deploy the
shell. Roll back the shell first. Existing clients do not need migration.

## Resource measurement

The old shell source specifies a 500 ms successful refresh interval: two snapshot
queries per second while healthy. The expected subscription change is zero
periodic queries and zero IPC timer wakes while idle, with one persistent client
and one wake socket pair instead. To compare on the same compositor binary, run
`cargo build --release --locked` and
`python3 scripts/measure-desktop-subscriptions.py` in the linked Villain checkout
on a Wayland desktop. Its `polling-baseline` mode emulates the old query cadence;
it is not a measurement of the older compositor binary. It starts isolated nested
compositors with temporary runtime/configuration directories.

On 2026-09-28, one five-second run measured:

| Mode | Queries | CPU ticks | RSS KiB start/end | Threads | FDs | Process context-switch delta |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 500 ms query baseline | 10 | +3 | 101624/101676 | 8 | 35 | +96 |
| Idle subscription | 0 | +2 | 105184/105208 | 8 | 37 | +27 |
| 40 workspace changes, 40 subscriber reconnects | 0 periodic | +1 | 104896/105252 | 8 | 37 | +203 |

The stress run retained 8 threads and 37 descriptors after churn and delivered
changes at 0.153 ms median, 0.421 ms p95 and 0.670 ms maximum from action
acknowledgement to snapshot receipt. Context switches are a whole-process proxy
for wakeups, not an exact IPC wakeup counter. CPU/RSS and context-switch samples
include nested rendering and are single-run observations; their deltas do not
establish a stable resource improvement. A direct-TTY baseline, exact IPC wakeup
counts, and long-running stress results remain unknown. The code-level
improvement established here is removal of the two-per-second snapshot request
and its successful-refresh timer.
