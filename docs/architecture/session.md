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
6. Supervise all children until shutdown or a bounded failure policy is reached.

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
true. `start_overview_service` defaults to false because the overview is an
exclusive layer and is normally launched transiently by Villain's keybind.
Setting it true is an explicit always-open mode. Runtime state and socket
identity are discovered at startup; they are not persisted in config.toml.

## Failure and shutdown behavior

Readiness, spawn, and partial-start failures terminate already-started
children before returning an error. SIGINT and SIGTERM set an atomic shutdown
flag; the supervisor sends SIGTERM, waits up to two seconds, then force-kills
and reaps any remaining direct child.

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
unchanged canonical config file; no migration deletes legacy values.
