# Desktop state subscriptions

Villain implements Knave's `subscribe` request on a dedicated connection.
The first response is a complete snapshot, registered atomically on the compositor
event loop. Later snapshots replace prior state and may coalesce intermediate
changes. Existing request/response clients are unaffected; no unsolicited frames
are written to query or action connections.

Relevant mutations mark desktop metadata dirty. After an event batch, Villain
compares workspace/window summaries and advances a session-local generation only
when they differ. Wayland title/app-ID callbacks and XWayland property notifications
are included. Cursor movement and pixel-only commits do not broadcast snapshots.
This does not implement live preview damage notifications.

A nonblocking listener accepts up to 16 clients. Each client uses one worker;
subscription workers block on socket readiness and a wake socket instead of a
timer. Each subscriber retains at most one pending frame; serialized frames
are shared and transmitted at no more than 16 MiB. Serialization may allocate
more transiently before checking the size. Queued requests are cancelled after
two seconds; requests already executing wait for their actual result. Socket
writes time out after two seconds. Per-client socket setup failures close only
that client. Completion notifications reap workers and close control descriptors;
shutdown removes sources, closes all client sockets and joins workers. The
compositor never waits for subscriber writes.

Knave, Villain, and Shell use the same request and snapshot shapes; update the
three packages together. No user configuration, socket path, or permissions
change occurs. See Knave's ADR 0002 for wire semantics.

## Measurement

After `cargo build --release --locked`, run
`python3 scripts/measure-desktop-subscriptions.py` on a Wayland desktop. The
script starts separate, isolated nested compositors and compares a client
querying every 500 ms with an idle subscription on the same new server binary.
This is a protocol-equivalent polling baseline, not a run of the old server.

One run on 2026-09-28 observed:

| Mode | Periodic queries / 5 s | CPU ticks | RSS KiB start/end | Threads | FDs | Context-switch delta |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Polling baseline | 10 | +3 | 101624/101676 | 8 | 35 | +96 |
| Idle subscription | 0 | +2 | 105184/105208 | 8 | 37 | +27 |
| 40 workspace changes and 40 subscriber reconnects | 0 | +1 | 104896/105252 | 8 | 37 | +203 |

The stress run returned to 8 threads and 37 descriptors. Action
acknowledgement to snapshot receipt was 0.153 ms median, 0.421 ms p95 and
0.670 ms maximum, with increasing generations. The five-second idle
subscription had no unsolicited data. These are single-run observations,
not a stable CPU or memory improvement claim. Whole-process context switches
are a wakeup proxy and include nested rendering; exact IPC wakeups, direct-TTY
behavior, old-server resource measurements, and long-running stress results
remain unknown. The expected deterministic change is removal of two periodic
snapshot requests per second while connected.
