# Knave Desktop Environment

Knave is the umbrella project for an independent Linux desktop environment.
This repository owns the desktop-wide contracts, configuration, session
lifecycle, and the long-term Rust/Cargo-first environment.

The compositor and shell remain separate processes and repositories:

| Repository | Responsibility |
| --- | --- |
| Knave | Settings, session supervisor, public desktop API, protocol, CLI |
| Knave Shell | Rust UI library, wgpu renderer, Wayland client, shell |
| Villain | Wayland compositor and window manager |

## Workspace

The current Knave Cargo workspace contains:

| Crate | Responsibility |
| --- | --- |
| knave-desktop-api | Versioned desktop IDs, snapshots, requests, events, and errors |
| knave-config | Typed schema, validation, preservation-aware TOML writes |
| knave-session | Compositor and shell startup, supervision, restart, and cleanup |
| knave | Configuration and environment control CLI |

Build and test it with:

    cargo fmt --all -- --check
    cargo test --workspace
    cargo clippy --workspace --all-targets -- -D warnings
    cargo build --workspace --release --locked

## CLI

The CLI exposes configuration and session operations:

    cargo run -p knave -- version
    cargo run -p knave -- session start
    cargo run -p knave -- session check
    cargo run -p knave -- config path
    cargo run -p knave -- config init
    cargo run -p knave -- config migrate
    cargo run -p knave -- config check
    cargo run -p knave -- config print

The public compositor control client is a separate installed binary:

    cargo run -p knavectl -- --help

After `knave session start`, query and control the running desktop with:

    knavectl snapshot
    knavectl workspaces
    knavectl dispatch workspace 2
    knavectl dispatch reload

session start loads ~/.config/knave/config.toml, starts the configured Villain
binary, waits for a newly-created wayland-* socket, and starts the configured
shell roles. SIGINT and SIGTERM stop the children with a bounded grace period.
Failed components are restarted with capped exponential backoff when
restart_on_failure is enabled; the current supervisor permits at most three
restart attempts per session.

## Configuration

The canonical target path is:

    ~/.config/knave/config.toml

The path can be overridden with KNAVE_CONFIG. If XDG_CONFIG_HOME is set, Knave
uses XDG_CONFIG_HOME/knave/config.toml.

Initialize and validate an isolated configuration:

    KNAVE_CONFIG=/tmp/knave/config.toml target/release/knave config init
    KNAVE_CONFIG=/tmp/knave/config.toml target/release/knave config check

The schema is versioned and written atomically. Known values are typed and
validated; unknown TOML values are preserved when known settings are changed.
Villain’s root-level compositor settings are projected into the typed
compositor section when needed, while the original TOML remains recoverable.
New writes use Knave-owned compositor settings.

See [config.example.toml](config.example.toml) and
[configuration.md](docs/architecture/configuration.md).

## Installation

The installer builds and installs the release CLI, public control client, and
session supervisor. It supports all three requested modes:

    scripts/install.sh --user
    scripts/install.sh --system
    scripts/install.sh --prefix /chosen/prefix

User installation places knave, knavectl, and knave-session in ~/.local/bin.
System installation uses /usr/local/bin and requests sudo when necessary.
Installation is explicit and never writes to /usr/local without --system.

## Architecture and change policy

- [Architecture index](docs/architecture/README.md)
- [Component boundaries](docs/architecture/component-boundaries.md)
- [Configuration](docs/architecture/configuration.md)
- [Session lifecycle](docs/architecture/session.md)
- [Versioning](docs/architecture/versioning.md)
- [CLI contract](docs/architecture/cli.md)
- [Build and packaging](docs/architecture/build-and-packaging.md)
- [Performance](docs/architecture/performance.md)
- [Change impact](docs/architecture/change-impact.md)
- [Agent instructions](AGENTS.md)

Before changing a shared contract, inspect every consumer in all three
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

Custom binding sets replace defaults. Append these entries to your existing
configuration if you use custom bindings; change the keys or signed step size
as desired:

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

Run `knavectl reload` after editing. Resizing is per workspace and lasts
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
Validate with `knave config check`, then apply with `knavectl reload` on the matching
Villain build. Initial compositor deployment requires a session restart.
