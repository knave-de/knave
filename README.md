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

The rewrite is in progress. This repository currently provides the first
working configuration and contract foundation; the unified session launcher
and the Rust/wgpu shell are not complete yet.

## Workspace

The current Knave Cargo workspace contains:

| Crate | Responsibility |
| --- | --- |
| knave-desktop-api | Versioned desktop IDs, snapshots, requests, events, and errors |
| knave-config | Typed schema, validation, preservation-aware TOML writes |
| knave | Configuration and environment control CLI |

Build and test it with:

    cargo fmt --all -- --check
    cargo test --workspace
    cargo clippy --workspace --all-targets -- -D warnings
    cargo build --workspace --release --locked

## CLI

The current CLI exposes configuration operations:

    cargo run -- version
    cargo run -- config path
    cargo run -- config init
    cargo run -- config check
    cargo run -- config print

The session commands will be added only when the supervisor has real readiness,
shutdown, restart, and rollback behavior. The CLI must not claim to start an
environment before that implementation exists.

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
Legacy Villain configuration migration is planned but is not implemented by
this foundation slice.

See [config.example.toml](config.example.toml) and
[configuration.md](docs/architecture/configuration.md).

## Installation

The installer builds a release binary and supports all three requested modes:

    scripts/install.sh --user
    scripts/install.sh --system
    scripts/install.sh --prefix /chosen/prefix

User installation places knave in ~/.local/bin. System installation uses
/usr/local/bin and requests sudo when necessary. Installation is explicit and
never writes to /usr/local without --system.

## Architecture and change policy

- [Architecture index](docs/architecture/README.md)
- [Component boundaries](docs/architecture/component-boundaries.md)
- [Configuration](docs/architecture/configuration.md)
- [Versioning](docs/architecture/versioning.md)
- [Build and packaging](docs/architecture/build-and-packaging.md)
- [Performance](docs/architecture/performance.md)
- [Change impact](docs/architecture/change-impact.md)
- [Agent instructions](AGENTS.md)

Before changing a shared contract, inspect every consumer in all three
repositories. Keep protocol, configuration, session, UI, compositor, and
installation changes in defined PRs with compatibility and rollback evidence.
