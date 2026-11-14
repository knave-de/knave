# Single executable, isolated processes, private IPC

Separate component executables made installation and child lookup vulnerable to
mixed builds. Separate Shell role processes remain useful failure boundaries;
component ownership does not require independently shipped executables.

## Decision

The workspace emits one `knave` executable. Session supervision, Villain, Shell
roles, desktop-control commands, services, and helpers are library entrypoints.
The supervisor and owned popup/helper launchers re-execute the running artifact
with private `--internal` modes. Existing bounded restart, reaping, cancellation,
and per-role resource lifetimes remain. No new watcher, worker, or queue is added.
Shell roles are not combined into one process in this change.

The documented CLI and its output are the external desktop-control interface.
`knave-desktop-api` is a non-published internal message/transport crate, despite
its historical name. Socket payloads, subscriptions, internal service messages,
and launch modes have no independently supported public compatibility promise.
All consumers, including the coordinated portal backend, update together.
Existing sockets remain transports; this is not a switch to inherited channels
or an authentication/access-control change. Standard Wayland, XDG portal,
notification, and Polkit D-Bus contracts remain intact.

## Configuration, rollout, and rollback

`session.compositor_binary` and `session.shell_binary` are removed from the typed
configuration, defaults, validation, and writer. Child identity comes from the
running executable. Settings writes preserve unknown source data, including
unrecognized former keys; those values cannot override process identity. No
schema version, old-format branch, or destructive migration is added.

Install the coordinated `knave` and portal-backend builds together; update
scripts to use `knave` instead of `knavectl`. The portal remains its own externally
activated service executable. Do not remove old installed files automatically
or replace the active desktop. Rollback restores the previous coordinated
packages and launch commands; user configuration and notification history remain.
Package versions remain workspace versions, without a separate IPC version.

## Verification

Check Cargo metadata for exactly one workspace binary target, CLI routing and
rejection of malformed arguments, supervisor clean exit/failure/restart/reaping,
wallpaper reconciliation, and private helper/popup launches. Run workspace
formatting, tests, Clippy, debug/release builds, staged installation, and portal
consumer checks. Exercise Overview, application launch, workspace switching,
reload, Shell recovery, and logout in the nested harness. Record resource samples
and a video; nested software rendering does not establish direct-TTY, hardware
GPU, real authentication, or sandboxed portal behavior.
