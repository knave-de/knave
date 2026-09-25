# Knave Agent Instructions

Knave is the umbrella repository for the Knave Desktop Environment.

Knave owns the user-facing configuration, settings, session lifecycle, build
orchestration, public desktop contracts, compatibility policy, and the Rust
Knave Shell/UI stack. Villain owns compositor and window-manager behavior.
Keep those responsibilities separate unless an explicit architecture decision
changes the boundary.

## Mandatory startup

Before editing:

1. Read this file and the nearest repository-specific `AGENTS.md`.
2. Read relevant architecture, configuration, build, and API documentation when
   it exists.
3. Inspect the complete owning module, not only the requested lines.
4. Search every consumer of the API, type, configuration key, binary, or
   protocol being changed.
5. Inspect repository status and preserve all existing user changes.
6. Classify the change as private, additive, behavioral, configuration,
   migratory, protocol, or breaking.
7. Write a short impact summary before implementing a non-local change.

Do not start implementation when the ownership or compatibility boundary is
unclear. Perform read-only investigation first.

## Component boundaries

The intended boundaries are:

- `knave-session`: session lifecycle and process supervision;
- `knave-settings`: typed configuration and settings operations;
- `knave-desktop-api`: public runtime desktop contracts;
- `knave-wayland`: Wayland client and layer-shell integration;
- `knave-renderer`: wgpu rendering;
- `knave-ui`: renderer-independent UI primitives;
- `knave-shell`: desktop-facing UI;
- Villain: compositor, layout, focus, window management, and input policy.

A change in one component must not reach through another component's private
implementation. Prefer explicit interfaces, adapters, and typed contracts over
shared mutable state or duplicated models.

Do not create another repository merely to split code that is still tightly
coupled. Split a repository only when it needs an independent release, API,
owner, or reuse outside Knave.

## Configuration ownership

The canonical user configuration is:

    ~/.config/knave/config.toml

Settings are initially a typed API and interface for reading and modifying
this file, not a second canonical persistence system.

Configuration must be schema-versioned, typed, validated, written atomically,
and migrated explicitly. Preserve user-defined values, bindings, and unknown
legacy data during migration. Never silently overwrite or delete legacy
Villain configuration.

Components consume typed configuration projections supplied by Knave. They do
not create competing user-facing configuration files. Runtime state belongs in
the Knave runtime directory or session service, not in persistent configuration.

## Versioning and compatibility

Track these independently:

- binary and library versions;
- configuration schema version;
- public desktop API version;
- private compositor protocol version;
- shell/UI protocol version.

Every public contract must define its version, compatibility range, additive
change behavior, breaking-change behavior, and migration or deprecation path.
`0.x` does not excuse undocumented breaking changes.

Do not remove or rename a public field, command, configuration key, protocol
message, or binary without finding every consumer, adding compatibility or
migration support where appropriate, updating tests and documentation, and
defining rollback behavior.

## Architecture impact analysis

For every architecture-affecting change, identify:

- what changed and who owns it;
- every consuming component and binary;
- affected public or private contracts;
- configuration and migration impact;
- required version changes;
- tests proving compatibility;
- rollout and rollback behavior.

For shared types, inspect all consumers before editing. For protocol changes,
inspect both sender and receiver. For configuration changes, inspect parser,
validator, writer, reload path, defaults, and migration. For lifecycle changes,
inspect startup, failure, shutdown, signal handling, process reaping, and stale
runtime files.

Do not merge a shared-library or protocol change only because its own package
passes tests.

## Build policy

The new Knave stack is Rust/Cargo-first. Use Cargo for Rust libraries,
binaries, tests, and workspace orchestration.

The current Qt shell is transitional and may continue using CMake until the
Rust/wgpu shell replaces it. Do not claim that CMake has been removed while it
still builds the shell. Transitional native builds should use Ninja rather than
direct Make invocations, isolated build directories, debug/release profiles,
and staged installation prefixes.

Build changes must check debug/release artifact paths, generated files,
dependency discovery, reproducibility, and installed-versus-workspace artifact
drift. Do not install to `/usr/local` implicitly.

## Code quality and comments

Prefer small, coherent modules with one clear responsibility. Do not perform
opportunistic rewrites, broad renames, or unrelated cleanup.

Runtime code must make ownership, lifecycle, cleanup, permissions, process
handling, shutdown, and compatibility behavior explicit. Use typed errors for
recoverable failures. Do not hide failures behind empty fallbacks or fabricated
success states.

Comments explain why, not obvious mechanics. Use short comments for invariants,
protocol sequencing, safety assumptions, compatibility reasons, and non-obvious
workarounds. Do not write essay-length comments or narrate the code.

## Documentation

README files contain the short user-facing build and run workflow. Architecture
documents contain boundaries, decisions, contracts, compatibility, migration,
and rollback. API documentation describes public behavior and versioning.

Documentation must be proportional to the change. Do not create long documents
or comments that merely repeat source code. Architecture changes should use one
concise ADR with context, decision, affected components, compatibility impact,
migration/rollback, and verification.

## Git and GitHub workflow

Create a focused branch for each task. Preserve unrelated dirty changes. Use
Conventional Commits, commit coherent slices, inspect the final diff, and
squash merge.

For cross-repository changes:

1. Create linked branches in affected repositories.
2. Define or preserve the shared contract.
3. Add compatibility support where needed.
4. Update all consumers.
5. Link the pull requests.
6. Verify the supported version combination.
7. Merge in dependency order.
8. Remove compatibility code only in a later cleanup change.

Cross-repository pull requests must describe scope, affected contracts,
consumers, version changes, compatibility, migration, verification, rollout,
and rollback.

## Verification and scope

After every implementation slice:

1. Format the code.
2. Run unit and relevant integration tests.
3. Run lint/static checks.
4. Build all affected components.
5. Run live smoke tests when behavior depends on Wayland, GPU, VT, or process
   lifecycle.
6. Inspect the final diff and run `git diff --check`.
7. Check generated and untracked files.
8. Report anything not actually verified.

Do not claim direct-TTY, GPU, installed-binary, Wayland, or reboot behavior from
compilation alone.

Do not combine configuration migration, session supervision, public API
creation, UI toolkit replacement, compositor behavior changes, build-system
replacement, packaging, and legacy-binary removal into one unreviewable change.
