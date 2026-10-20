# Clean compositor exit ends the session

The session supervisor previously restarted Villain after every exit, so
`knavectl dispatch quit` brought the desktop back when failure recovery was enabled.

Knave now treats a successful compositor exit as logout: stop and reap shell
roles and wallpaper, cancel wallpaper reconciliation, and return success.
Nonzero and signal exits retain the configured bounded recovery policy. A shell
exit alone still invokes shell recovery; compositor status is checked again
before respawning shells after backoff because Wayland disconnection can precede
compositor exit.

This changes private lifecycle behavior in `knave-session`, consumed by both
`knave session start` and `knave-session start`. Villain already exits successfully
on Quit. No public API, configuration, package version or migration changes are
needed. Rollback restores the previous supervisor; canonical configuration is
unchanged.

Subprocess lifecycle regressions cover successful exits with recovery enabled
and disabled, nonzero exits, signal exits, restart limits and child reaping.
Coordinated bundle checks and an isolated Winit quit exercise validate the actual
component set; direct TTY behavior requires separate human review.
