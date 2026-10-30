# Session lifecycle

Knave is the desktop environment entrypoint. The session supervisor owns
process lifecycle; it does not implement compositor policy, window management,
rendering, or shell composition.

## Startup

1. Load and validate ~/.config/knave/config.toml through knave-config.
2. Snapshot existing wayland-* Unix sockets in XDG_RUNTIME_DIR.
3. Start the configured compositor binary with KNAVE_SESSION=1. The backend
   field selects Villain's production TTY path or nested Winit path.
4. Wait for a new compositor-owned wayland-* socket, while checking that the
   compositor has not exited.
5. Start each configured shell role with WAYLAND_DISPLAY set to that socket.
6. Wait briefly for Villain's XWayland readiness status when startup commands
   are configured.
7. Start each configured `session.startup` command with the session displays.
8. Supervise session components until shutdown or a bounded failure policy is reached.

The supervisor does not attach to GNOME, KDE, Qt, or another host desktop.
Winit is a nested development and test backend; direct TTY/DRM is the
production path.

## Configuration

The [session] table is owned by Knave:

- backend: auto, tty, or winit;
- compositor_binary: executable used for Villain;
- shell_binary: executable used for Knave Shell; and
- restart_on_failure: whether failed components may be restarted.

The [shell] table controls session-owned shell roles. `start_bar` is normally
true. `start_overview_service` defaults to true and starts one persistent,
initially hidden Overview process for the Wayland session. Villain toggles its
visibility; it does not spawn Overview processes. Setting the option false
disables that session service. Runtime state and socket identity are discovered
at startup; they are not persisted in config.toml.

The supervisor starts one `polkit-agent` shell role for direct sessions and probes
`--supports-polkit-agent` before spawning shell children. It shares their bounded
restart and cleanup policy. The native agent registers for the current logind
session; a missing system service or logind session is logged without ending
the desktop. Winit sessions skip this role; Auto with an inherited display also
skips it to protect the enclosing desktop. Select `backend = "tty"` explicitly
when launching directly with inherited display variables. See [ADR 0015](decisions/0015-native-polkit-agent.md).

Each `[[session.startup]]` entry supplies a direct executable and argument
array. Entries start after the shell roles have been started, inherit the
session environment, receive the session's `WAYLAND_DISPLAY`, and do not inherit
`WAYLAND_SOCKET` from a host session. Knave waits up to five seconds for
Villain's XWayland status; when ready, its `DISPLAY` value is passed to startup
commands. When XWayland is unavailable or times out, Wayland startup continues
without a `DISPLAY` value. A spawn failure is logged and startup continues; an
entry that exits is reaped without affecting the session.
These processes are stopped on logout and relaunched after a compositor
restart. They are not restarted when they exit on their own, and they do not
participate in the shell restart policy. Like the other session children, only
the direct child is tracked. Configuration changes apply on the next session
start.

Before starting shell roles, the supervisor probes the configured binary with
`--supports-overview-service`. A supported Shell returns exit status 0 without
opening Wayland or GPU resources; unknown/unsupported flags return nonzero. A
failed or two-second timed-out probe stops startup with an upgrade diagnostic
and cleans up the compositor. The probe is repeated on shell restart, reaped
on every path, and adds no ongoing worker. Disabling the Overview role skips
the probe. Install the matching Shell before starting the new session manager;
older Shell binaries are rejected instead of mapping an exclusive overlay.

## Failure and shutdown behavior

Readiness, spawn, and partial-start failures terminate already-started
children before returning an error. SIGINT and SIGTERM set an atomic shutdown
flag; the supervisor sends SIGTERM, waits up to two seconds, then force-kills
and reaps any remaining direct child.

A successful compositor exit (including `knavectl dispatch quit`) ends the
session successfully, stops and reaps the shell roles and wallpaper, and never
restarts the compositor, regardless of `restart_on_failure`.

A compositor or shell failure stops its sibling shell processes before restart.
Restarts use 100, 200, and 400 millisecond delays and are capped at three
attempts per session. A failure after the cap returns a non-zero error and
does not claim that the desktop is running. The current implementation
supervises direct children; process-group supervision and service-manager
integration are separate future work.

## Resource limits

The loop has one supervisor thread and a 50 millisecond state poll only while
the session is active. Wayland socket discovery is bounded to startup and
compositor restart. There is no unbounded queue, watcher, worker pool, or
parallel restart fan-out. Shell startup is sequential and partial startup is
cleaned up.

Future lifecycle changes must record CPU, memory, process, descriptor, wakeup,
retry, and shutdown effects as required by the performance policy.

## Verification and rollback

Unit tests cover socket filtering and restart backoff. Cargo tests, strict
Clippy, release builds, installer checks, and diff checks are required for
changes to this crate.

Compilation does not prove direct TTY, DRM, Wayland, GPU, or installed-binary
behavior. Before rollout, exercise knave session check, an isolated Winit
session, and a real logged-in TTY session with signal and child-failure tests.
Rollback is the prior Knave/Villain/Shell version combination and the
unchanged canonical config file; no migration deletes source values.
