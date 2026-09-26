# CLI contract

Knave owns the public command surface for the complete desktop environment.
Villain-local control commands are not a second interface; the old
`villainctl` binary is replaced by `knavectl`.

## Environment CLI

The `knave` binary owns configuration and session operations:

| Command | Behavior |
| --- | --- |
| `knave version` | Print the Knave binary and public desktop API version. |
| `knave session start` | Load the canonical configuration and supervise Villain and configured Shell roles. |
| `knave session check` | Validate the canonical configuration and report its backend and component binaries. |
| `knave config path` | Print the resolved configuration path. |
| `knave config init` | Create a default configuration; fail if the target already exists. |
| `knave config migrate` | Materialize root-level compositor settings under `[compositor]` while preserving the source keys. |
| `knave config check` | Parse and validate the configuration without writing it. |
| `knave config print` | Print the preserved editable TOML document. |

Use `KNAVE_CONFIG` for an isolated file. Otherwise Knave resolves
`$XDG_CONFIG_HOME/knave/config.toml`, falling back to
`~/.config/knave/config.toml`.

The compatibility binary `knave-session` provides `start` and `check` for
scripts that need a dedicated session executable. It uses the same
`knave-session` library and configuration path as `knave session ...`.

## Desktop control CLI

`knavectl` sends one request to the active Knave desktop socket and prints the
JSON response. `KNAVE_SOCKET` overrides socket discovery for isolated tests.

| Command | Request |
| --- | --- |
| `knavectl reload` | Reload compositor-owned settings. |
| `knavectl dispatch reload` | Explicit dispatch spelling for reloading compositor-owned settings. |
| `knavectl snapshot` | Print the complete desktop snapshot. |
| `knavectl windows` | List window summaries. |
| `knavectl workspaces` | List workspace summaries. |
| `knavectl active-window` | Print the focused window, if any. |
| `knavectl active-workspace` | Print the active workspace. |
| `knavectl version` | Query the desktop API and compositor version. |
| `knavectl dispatch close` | Close the focused window. |
| `knavectl dispatch minimize` | Minimize the focused window. |
| `knavectl dispatch restore-minimized` | Restore the most recently minimized window. |
| `knavectl dispatch workspace <1-10>` | Focus a workspace. |
| `knavectl dispatch focus-window <id>` | Focus a non-minimized window. |
| `knavectl dispatch restore-window <id>` | Restore and focus a window. |
| `knavectl dispatch exec <program> [args...]` | Spawn a compositor-managed process. |
| `knavectl dispatch quit` | Ask the compositor to shut down. |

The `dispatch reload` spelling remains accepted as an explicit dispatch form;
`knavectl reload` is the short form. Unknown commands, missing arguments, trailing arguments for fixed-arity commands,
invalid numeric IDs, and workspace IDs outside 1-10 return a non-zero exit status
and a usage diagnostic.

## Compatibility policy

Adding a command is additive. Renaming or removing a command requires consumer
discovery, a documented replacement, and a later cleanup change. A command
that changes its JSON response or dispatch semantics is a public desktop API
change and must update the API versioning, both client and compositor tests,
and the linked Knave/Villain/Shell pull requests.
