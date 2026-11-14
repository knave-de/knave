# Knave Desktop Environment

Knave is the umbrella project for an independent Linux desktop environment.
This repository owns the desktop-wide contracts, configuration, session
lifecycle, and the long-term Rust/Cargo-first environment.

Shell, Villain, and the desktop contracts share this repository and Cargo
workspace. Only the following remain separate:

| Repository | Responsibility |
| --- | --- |
| Knave (this repository) | Desktop API, configuration, session, CLI, shell, compositor |
| [Knave UI](https://github.com/knave-de/knave-ui) | Renderer-independent UI toolkit, assets, wgpu renderer |
| [xdg-desktop-portal-knave](https://github.com/knave-de/xdg-desktop-portal-knave) | Desktop portal backend |

## Workspace

The Cargo workspace contains:

| Crate | Responsibility |
| --- | --- |
| knave-desktop-api | Private desktop IDs, snapshots, requests, events, and errors |
| knave-config | Typed schema, validation, preservation-aware TOML writes |
| knave-session | Compositor and shell startup, supervision, restart, and cleanup |
| knave | Configuration and environment control CLI |
| knavectl | Desktop control client library |
| knave-portal-api | Portal-facing IPC types |
| knave-capture-protocol | Screenshot and screen-cast protocol types |
| knave-apps | XDG application catalog and search |
| knave-icons | Freedesktop icon lookup and rasterization |
| knave-shell-ui | Shell widgets and layout |
| knave-wayland | Wayland client and layer-shell integration |
| knave-shell | Desktop surfaces and application integration |
| villain | Wayland compositor and window manager |

The workspace emits one executable: `target/{debug,release}/knave`.
Session, compositor, Shell, services, and helpers run as separate processes of
that executable. Component crates retain their ownership boundaries. Install
with `scripts/install.sh --user` or an explicit `--prefix PATH`.
Knave's internal IPC is private; external tools use the `knave` CLI or standard
Wayland/D-Bus interfaces. See [the consolidation decision](docs/architecture/decisions/0017-single-executable.md).

`knave-ui` and `knave-renderer` are consumed from the standalone Knave UI
repository at a pinned revision.

Build and test it with:

    cargo fmt --all -- --check
    cargo test --workspace
    cargo clippy --workspace --all-targets -- -D warnings
    cargo build --workspace --release --locked

## Development workflow

Isolate changes in a plain Git worktree; the source checkout stays untouched:

    git worktree add -b my-change ../knave-my-change master

The former `knave-dev` managed workspace and bundle tooling is still present
under `scripts/` but is being replaced; prefer worktrees for branch isolation
until the phase 1 tooling lands. For agent-controlled feature testing, MP4
demonstrations, and interactive testing over an SSH/VNC connection, see the
[desktop test harness](docs/desktop-testing.md).

## CLI

The CLI exposes configuration and session operations:

    cargo run -p knave -- version
    cargo run -p knave -- session start
    cargo run -p knave -- session check
    cargo run -p knave -- config path
    cargo run -p knave -- config init
    cargo run -p knave -- config check
    cargo run -p knave -- config print

The same executable provides desktop control:

    cargo run -p knave -- --help

After `knave session start`, query and control the running desktop with:

    knave snapshot
    knave workspaces
    knave dispatch workspace 2
    knave dispatch reload

`knave session start` loads ~/.config/knave/config.toml, starts its own
compositor process mode, waits for a newly-created wayland-* socket, and starts
its own Shell process modes followed by any `[[session.startup]]` commands. Startup commands
are argument arrays and run without a shell; failed commands are logged and do
not stop the session. SIGINT and SIGTERM stop tracked children with a bounded
grace period.
Failed components are restarted with capped exponential backoff when
restart_on_failure is enabled; the current supervisor permits at most three
restart attempts per session. A successful compositor exit, including
`knave dispatch quit`, stops the session without restarting it.

## Configuration

The canonical target path is:

    ~/.config/knave/config.toml

The path can be overridden with KNAVE_CONFIG. If XDG_CONFIG_HOME is set, Knave
uses XDG_CONFIG_HOME/knave/config.toml.

Initialize and validate an isolated configuration:

    KNAVE_CONFIG=/tmp/knave/config.toml target/release/knave config init
    KNAVE_CONFIG=/tmp/knave/config.toml target/release/knave config check

Configuration is written atomically. Known values are typed and validated;
unknown TOML values are preserved when known settings are changed. Compositor
settings belong under `[compositor]`. The unreleased configuration format has
no schema version or legacy migration paths.

See [config.example.toml](config.example.toml) and
[configuration.md](docs/architecture/configuration.md).

## Installation

The installer builds and installs the release `knave` executable and desktop
data. Choose a user, system, or explicit prefix:

    scripts/install.sh --user
    scripts/install.sh --system
    scripts/install.sh --prefix /chosen/prefix

User installation places only `knave` in `~/.local/bin`.
System installation uses /usr/local/bin and requests sudo when necessary.
Installation is explicit and never writes to /usr/local without --system.

## Architecture and change policy

- [Architecture index](docs/architecture/README.md)
- [Component boundaries](docs/architecture/component-boundaries.md)
- [Configuration](docs/architecture/configuration.md)
- [Session lifecycle](docs/architecture/session.md)
- [Package versions](docs/architecture/versioning.md)
- [CLI contract](docs/architecture/cli.md)
- [Build and packaging](docs/architecture/build-and-packaging.md)
- [Performance](docs/architecture/performance.md)
- [Change impact](docs/architecture/change-impact.md)
- [Agent instructions](AGENTS.md)

Before changing a shared contract, inspect every consumer in the affected
repositories. Keep protocol, configuration, session, UI, compositor, and
installation changes in defined PRs with compatibility and rollback evidence.

### Master/stack resizing

With the matching Villain layout-resize implementation, edit the existing
`[compositor]` table in `~/.config/knave/config.toml`:

```toml
[compositor]
master_percent = 60 # 60% master, 40% stack; default 50, valid range 10..90
```

Left-drag within five logical pixels of the main vertical divider. The cursor
changes to a horizontal resize cursor. Default shortcuts are `MOD+Ctrl+Left`
(shrink master by 5 percentage points), `MOD+Ctrl+Right` (grow by 5), and
`MOD+Ctrl+R` (reset to the configured default). Each key press makes one step.
`MOD` is the configured modkey, normally Super.

Custom bindings override the defaults with the same key combination. Append
these entries to your existing configuration to use them; change the keys or
signed step size as desired:

```toml
[[compositor.bind]]
keys = "MOD+CTRL+LEFT"
dispatch = "resize-master"
args = ["-5"]

[[compositor.bind]]
keys = "MOD+CTRL+RIGHT"
dispatch = "resize-master"
args = ["5"]

[[compositor.bind]]
keys = "MOD+CTRL+R"
dispatch = "reset-master"
```

Run `knave reload` after editing. Resizing is per workspace and lasts
until reset or restart; reload preserves manual overrides. The percentage is
clamped to 10–90%. Resize actions do nothing with fewer than two tiled windows
or while a maximized/fullscreen window covers the layout. Floating windows and
individual stack dividers are unchanged. These are Villain keybinding actions,
not new desktop IPC commands.

### Window appearance

Configure independent gap/border edges, corner radii, focused/unfocused opacity,
border colors, per-side shadows and background blur in `[compositor.appearance]`.
Maximized effects are individually opt-in; fullscreen always bypasses effects.
See the [configuration reference and example](docs/architecture/window-appearance.md).
Validate with `knave config check`, then apply with `knave reload` on the matching
Villain build. Initial compositor deployment requires a session restart.

### Desktop portals

`[portal]` in the canonical configuration controls startup enablement and desktop
appearance. Monitor selection, screenshot consent and Stop sharing use Knave
Shell's native UI. Install the matching backend with
`../xdg-desktop-portal-knave/scripts/install.py --user`; its activation files and
routing defaults are user-local. Start a fresh direct Knave session to activate
Knave routing. Nested sessions preserve the host activation environment. See
[the portal decision](docs/architecture/decisions/0006-desktop-portals.md) for
contracts, supported capabilities, limits, verification and rollback.

## Workspace swipes

Three-finger horizontal touchpad swipes move workspace scenes with your fingers.
Left advances and creates a workspace at the dynamic tail when needed; right
goes back and stops at the first workspace. Release beyond a quarter of the
output width commits; shorter or cancelled swipes settle back.
Panels and the cursor stay stationary. See the
[gesture contract](docs/architecture/decisions/0010-workspace-swipes.md).
Override entries under `[[compositor.gesture.bind]]` by fingers and direction;
`action = "none"` disables the matching compositor gesture. These are Villain
actions, not knave commands. Reload applies validated changes atomically.

To skip empty slots when cycling with next/previous shortcuts or swipes, set
`skip_empty = true` under `[compositor.workspace]` and reload. It defaults to
false; direct number/bar selection still reaches empty slots. Minimized windows
count as occupied. The default keeps one predefined workspace and one dynamic
workspace. Set `predefined` to a larger positive number to keep a contiguous
prefix permanently; the compositor retains all workspaces through the highest
occupied slot and one empty workspace after it. Empty slots are removed only
from the end, never between retained workspaces.

Notifications are provided by the session's built-in service when no other daemon
already owns `org.freedesktop.Notifications`. The first owner wins; Knave neither
replaces it nor waits to take over. History and Do not disturb survive restarts
in `$XDG_STATE_HOME/knave/notifications.sqlite3`. Defaults keep seven days or 200
entries. The shell bell opens history and shows the unread count.

```toml
[notifications]
enabled = true
history_limit = 200
history_days = 7
banner_timeout_ms = 6000
max_banners = 3
critical_in_dnd = false

[notifications.app."org.example.App"]
enabled = true
banners = false
```

Notifications also show attached raw images and local PNG/JPEG/WebP image hints.
Bounded previews are kept with history; the app icon remains separate.

Restart the session to apply edited notification settings. Per-app banner controls
in the center apply immediately. See [notification architecture](docs/architecture/decisions/0012-notifications.md).
