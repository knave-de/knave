# ADR 0001: Knave environment boundaries

- Status: Accepted
- Date: 2026-09-25

## Context

The desktop environment currently spans Knave, a shell implementation, and
Villain. Configuration, lifecycle, UI, and compositor responsibilities can
drift when each repository treats itself as the whole environment.

## Decision

Knave is the umbrella owner for user-facing configuration, settings, session
lifecycle, public desktop contracts, build orchestration, and the Rust shell/UI
stack. Villain remains responsible for compositor and window-manager behavior.
Repository boundaries are preserved unless an independent release, API owner,
or reuse case justifies a new repository.

The canonical configuration remains `~/.config/knave/config.toml`. The settings
API operates on that file; it does not introduce a second canonical store.
Rust/Cargo is the supported build model, and the Rust/wgpu shell is the
supported shell implementation. Knave is independent of GNOME, KDE, Qt, and
other host desktop environments.

## Consequences

Changes cross an explicit typed contract or adapter. Configuration migration,
session supervision, public API creation, UI replacement, compositor behavior,
build replacement, packaging, and legacy-tool removal are separate reviewed
changes. Compatibility and rollback are part of each change's design.

## Non-goals

This ADR does not define the compositor's private implementation or replace the
canonical Knave configuration with a second store. Removal of a public command
or configuration key still requires consumer discovery, migration support, and
verified rollback behavior.
