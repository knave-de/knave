# CLI contract

Knave owns the public command surface for the complete desktop environment.
Villain-local control commands are not a second interface.

## Environment CLI

The `knave` binary owns configuration and session operations:

| Command | Behavior |
| --- | --- |
| `knave version` | Print the Knave package version. |
| `knave session start` | Load the canonical configuration and supervise Villain and configured Shell roles. |
| `knave session check` | Validate the canonical configuration and report its backend and component binaries. |
| `knave config path` | Print the resolved configuration path. |
| `knave config init` | Create a default configuration; fail if the target already exists. |
| `knave config check` | Parse and validate the configuration without writing it. |
| `knave config print` | Print the preserved editable TOML document. |

Use `KNAVE_CONFIG` for an isolated file. Otherwise Knave resolves
`$XDG_CONFIG_HOME/knave/config.toml`, falling back to
`~/.config/knave/config.toml`.

## Desktop control CLI

Desktop-control invocations of `knave` send one request to the active desktop
and print the JSON response. `KNAVE_SOCKET` overrides socket discovery; when `WAYLAND_DISPLAY`
is absent, it discovers the socket if exactly one live Knave session is present.
If multiple sessions are live, set `WAYLAND_DISPLAY` or `KNAVE_SOCKET` to select one.

| Command | Request |
| --- | --- |
| `knave reload` | Reload compositor-owned settings. |
| `knave dispatch reload` | Explicit dispatch spelling for reloading compositor-owned settings. |
| `knave snapshot` | Print the complete desktop snapshot. |
| `knave windows` | List window summaries. |
| `knave workspaces` | List workspace summaries. |
| `knave desktop-version` | Query the running compositor package identity and version. |
| `knave devices` | List input devices with their enabled state, capabilities, and matched `[[compositor.device]]` rule index. |
| `knave active-window` | Print the focused window, if any. |
| `knave active-workspace` | Print the active workspace. |
| `knave dispatch move-window [--target TARGET] [--window ID]` | Begin interactive movement/rearrangement; target selectors are mutually exclusive. |
| `knave dispatch resize-window [--target TARGET] [--window ID] [--edge EDGE]` | Begin floating resize or tiled master split adjustment. |
| `knave dispatch toggle-floating` | Toggle the focused window between tiled and floating. |
| `knave dispatch close` | Close the focused window. |
| `knave dispatch maximize` | Maximize the focused window within the usable workspace area. |
| `knave dispatch unmaximize` | Restore its underlying tiled/floating layout without changing fullscreen or minimization. |
| `knave dispatch toggle-maximize` | Toggle the focused window's saved maximization state. |
| `knave dispatch minimize` | Minimize the focused window. |
| `knave dispatch restore-minimized` | Restore the most recently minimized window. |
| `knave dispatch workspace <1-10>` | Focus a workspace. |
| `knave dispatch focus-window <id>` | Focus a non-minimized window. |
| `knave dispatch restore-window <id>` | Restore and focus a window. |
| `knave dispatch exec <program> [args...]` | Spawn a compositor-managed process. |
| `knave dispatch quit` | Ask the compositor to shut down. |

The `dispatch reload` spelling remains accepted as an explicit dispatch form;
`knave reload` is the short form. Unknown commands, missing arguments, trailing arguments for fixed-arity commands,
invalid numeric IDs, and workspace IDs outside 1-10 return a non-zero exit status
and a usage diagnostic.

## Internal transport boundary

CLI commands and their documented output are the external control interface.
The Unix-socket requests, subscriptions, service messages, and `--internal`
process modes are private implementation details, updated together. The crate
name `knave-desktop-api` is internal and does not promise a public Rust/wire API.
Standard Wayland and D-Bus interfaces retain their external contracts.

Internal payload changes require consumer discovery, coordinated
updates in Knave, Villain, Shell, and portal components that use them, and tests
for the resulting package set.

Interactive commands resolve their target once (focused by default), acknowledge
startup immediately, and continue after the CLI disconnects. Move/resize use
pointer motion; left-button release or Enter commits and Escape cancels. Explicit
edges apply to floating windows and accept top, bottom, left, right, top-left,
top-right, bottom-left, bottom-right. Without an edge, resize selects the corner
nearest the initial pointer. Background, minimized, maximized/fullscreen targets
and competing grabs are rejected. There is no cancellation IPC command.
