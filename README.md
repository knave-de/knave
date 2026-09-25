# Knave Desktop Environment

Knave is the umbrella project for a cohesive Linux desktop environment. This
repository owns desktop-wide contracts and the long-term Rust/Cargo-first shell
direction. The compositor and current shell are developed separately.

## Repository role

Knave owns:

- user-facing configuration and typed settings;
- session lifecycle and process supervision;
- versioned public desktop contracts;
- build and release orchestration; and
- the Rust shell, renderer, UI, and Wayland component boundaries.

Villain owns compositor and window-manager behavior: surfaces, outputs, input,
focus, layout, workspaces, activation, and composition. Knave Shell owns
desktop-facing presentation and interaction.

See [component boundaries](docs/architecture/component-boundaries.md) and
[ADR 0001](docs/architecture/decisions/0001-knave-environment-boundaries.md).

## Repository map

| Repository | Current responsibility |
| --- | --- |
| Knave | Umbrella contracts, architecture, settings, session, and future Rust shell |
| Knave Shell | Current Qt/QML shell, Rust shell core, and layer-shell plugin |
| Villain | Rust/Smithay compositor and window manager |

Knave currently contains architecture and governance documentation. It is not
yet a root Cargo workspace and has no root runtime binary.

## Current implementation state

The working system is transitional:

- Villain is a Rust/Smithay compositor with nested Winit and direct TTY
  backends.
- Knave Shell is a Qt Quick application with a Rust core and a private Qt
  Wayland layer-shell plugin.
- Shell state crosses the repository boundary through the versioned Villain
  IPC contract.
- The planned Rust/wgpu shell, unified session environment, Knave-owned
  settings writer, and replacement for villainctl are not complete.
- villainctl remains a transitional developer client.

Do not describe the target Rust/wgpu environment or Knave configuration
migration as implemented until the corresponding runtime and migration tests
exist.

## Configuration

The target canonical configuration is:

    ~/.config/knave/config.toml

Knave settings will own typed parsing, validation, atomic writing, schema
migration, and compatibility preservation. Runtime state belongs in the
session runtime directory, not persistent configuration.

The current Villain implementation still reads its legacy configuration path:
~/.config/villain/config.toml, or the VILLAIN_CONFIG override. That reader is
compatibility behavior during migration. Do not add another user-facing
configuration store.

See [configuration.md](docs/architecture/configuration.md).

## Build and run

There is no root build command yet. Build the current components from their
repositories:

    # Villain
    cd ../abhiwm
    cargo build --workspace --locked

    # Knave Shell
    cd ../knaveshell
    cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Debug
    cmake --build build

The component READMEs document runtime prerequisites, backend selection,
installation, and live verification. Compilation alone does not verify a real
Wayland session, GPU, VT, or installed startup.

## Documentation

- [Architecture index](docs/architecture/README.md)
- [Component boundaries](docs/architecture/component-boundaries.md)
- [Configuration](docs/architecture/configuration.md)
- [Versioning and compatibility](docs/architecture/versioning.md)
- [Build and packaging](docs/architecture/build-and-packaging.md)
- [Performance and resource usage](docs/architecture/performance.md)
- [Change-impact procedure](docs/architecture/change-impact.md)

These documents are concise design contracts, not a claim that every target
component already exists.

## Cross-repository changes

Before changing a shared type, configuration key, binary, protocol, lifecycle
path, or build target:

1. identify the owner and every consumer;
2. record compatibility, migration, version, and performance impact;
3. update both sides of the contract;
4. verify in dependency order; and
5. document rollout and rollback behavior.

Read [AGENTS.md](AGENTS.md) before making changes.
