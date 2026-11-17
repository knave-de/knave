# Session window-control policy

Knave Session supplies a mandatory `org.gnome.desktop.wm.preferences/button-layout`
value of `':'` for its desktop. An ordinary startup write could be changed again
and would modify the user's preferences in other desktops. Instead, Session
compiles one locked dconf database into its private runtime directory and
appends it to a copy of the inherited dconf profile. Other keys and inherited
profile databases retain their normal behavior. A runtime GTK settings fallback
is appended after inherited system config directories for GTK's no-portal path;
user config remains higher precedence. This is derived runtime policy, not
another canonical settings store.

All owned compositor, Shell, wallpaper, and startup processes receive the
profile through explicit command environments with `GSETTINGS_BACKEND=dconf`.
Villain supplies these variables to application launches and, only in direct
sessions, D-Bus/systemd activation. Nested sessions do not alter the host's
activation environment. Already-running apps and programs deliberately launched
outside that environment do not acquire the policy retroactively. This locks
the GSettings key, not application-specific header implementations or GTK
property overrides. For GTK 3/4 sessions without a Wayland settings portal,
Session appends runtime-owned `gtk-3.0/settings.ini` and `gtk-4.0/settings.ini`
defaults through `XDG_CONFIG_DIRS`. It leaves `XDG_CONFIG_HOME` and its files
intact. GTK's Wayland settings portal takes precedence over this fallback, and
explicit application header-bar properties can still show client controls.
Custom application chrome is not inferred or cropped.

The new runtime dependency is the `dconf` compiler. A missing compiler, unreadable/oversized
inherited profile, compiler failure, cancellation, or five-second timeout aborts
startup before desktop children launch. The compiler is always reaped. There
is no monitor, watcher, extra persistent process, or periodic settings write.
Existing restart attempts reuse the same active profile.

Session shutdown removes the private locked database and profile. Direct
activation records that it imported the profile, allowing Session to restore
activation to the original databases and config directories first. Because
D-Bus cannot unset an activation variable, one atomically written, bounded
fallback profile remains
in `XDG_RUNTIME_DIR/knave-dconf-base-profile` until the login runtime is removed.
It contains no Knave lock. If activation restoration fails, the error is logged
and the active settings directory is retained to avoid dangling activation
paths. Abrupt session SIGKILL cannot run cleanup; the login runtime removes
remaining files.
There is no configuration schema, wire change, package bump, or persistent
preference migration. Rollback is the prior coordinated build and a new session.

Unit tests check the locked value, rejected writes, unrelated-key writability,
unchanged underlying user value, inherited profiles, compiler errors, and
runtime cleanup. Lifecycle fixtures cover child environment propagation,
restart, failure, and shutdown cleanup. Nested GTK smoke tests attempt `:close`
and then exercise both Evince and Foliate. Direct-TTY activation restoration
requires hardware-session verification; nested tests cannot establish it.
