#!/bin/sh
# Run against the compositor selected by WAYLAND_DISPLAY / XDG_RUNTIME_DIR.
set -eu
probe_dir=$(mktemp -d /tmp/villain-frame-probe.XXXXXX)
trap 'rm -rf "$probe_dir"' EXIT HUP INT TERM
protocols=$(pkg-config --variable=pkgdatadir wayland-protocols)
wayland-scanner client-header "$protocols/stable/xdg-shell/xdg-shell.xml" "$probe_dir/xdg-shell-client-protocol.h"
wayland-scanner private-code "$protocols/stable/xdg-shell/xdg-shell.xml" "$probe_dir/xdg-shell-protocol.c"
cc -Wall -Wextra -Werror -I"$probe_dir" "$(dirname "$0")/frame-callback.c" "$probe_dir/xdg-shell-protocol.c" $(pkg-config --cflags --libs wayland-client) -o "$probe_dir/probe"
"$probe_dir/probe"
