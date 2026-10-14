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
- `knave-config`: typed configuration and settings operations;
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
source data during migration. Never silently overwrite or delete source
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

The Knave stack is Rust/Cargo-first. Use Cargo for libraries, binaries, tests,
and workspace orchestration across the environment. The Rust/wgpu shell is the
supported shell implementation; no Qt/CMake build is part of the supported
runtime.

Build changes must check debug/release artifact paths, generated files,
dependency discovery, reproducibility, and installed-versus-workspace artifact
drift. Do not install to `/usr/local` implicitly.

## Code quality and comments

Prefer small, coherent modules with one clear responsibility. Do not perform
opportunistic refactors, broad renames, or unrelated cleanup.

Runtime code must make ownership, lifecycle, cleanup, permissions, process
handling, shutdown, and compatibility behavior explicit. Use typed errors for
recoverable failures. Do not hide failures behind empty fallbacks or fabricated
success states.

Comments explain why, not obvious mechanics. Use short comments for invariants,
protocol sequencing, safety assumptions, compatibility reasons, and non-obvious
workarounds. Do not write essay-length comments or narrate the code.

## Performance and resource usage

Before adding a watcher, timer, event subscription, background task, thread,
process, cache, buffer, or parallel work, review its resource behavior.

- Give every activity an explicit lifetime, cancellation path, and cleanup
  path on reload, failure, and shutdown.
- Bound concurrency, queue size, memory growth, retries, and spawned work. Do
  not use unbounded task or thread creation as a default parallelism strategy.
- Prefer event-driven notifications, debouncing, and backoff over busy loops or
  high-frequency polling. Check that a simple watcher cannot wake continuously.
- Measure CPU, resident memory, threads, processes, file descriptors, and
  wakeups under idle, normal, and stress workloads when the change can affect
  them.
- Record a baseline and expected change. If no baseline exists, mark the
  behavior unknown and measure it before treating the change as complete.

Compilation and functional tests do not establish acceptable performance.

## Documentation

README files contain the short user-facing build and run workflow. Architecture
documents contain boundaries, decisions, contracts, compatibility, migration,
and rollback. API documentation describes public behavior and versioning.

Documentation must be proportional to the change. Do not create long documents
or comments that merely repeat source code. Architecture changes should use one
concise ADR with context, decision, affected components, compatibility impact,
migration/rollback, and verification.

## Commit identity and attribution

- Use the human contributor's established Git identity as author and committer
  for agent-created commits. The identity belongs to the person contributing
  the work, not necessarily the repository owner. Never hardcode the owner's
  identity or replace another human contributor's attribution. If the correct
  human identity cannot be established, ask the contributor before committing.
- Before committing, verify the effective identities with `git var GIT_AUTHOR_IDENT`
  and `git var GIT_COMMITTER_IDENT`, including environment and command overrides.
  If they differ, set the correct repository-local identity and recheck. Never
  invent an identity or use Codex, Claude, OpenAI, a bot, or any AI agent identity.
- Never add assistant attribution in `Co-authored-by`, `Signed-off-by`, or other
  commit-message trailers. Do not add co-authors without the user's explicit
  instruction. Preserve legitimate existing human and upstream attribution.
- After each commit or amendment, inspect its complete author, committer, and
  message, including all trailers, with `git show --no-patch --format=fuller HEAD`.
  Before pushing or opening a PR, audit every outgoing commit, not only the tip.
  Correct unintended attribution before publishing.
- Before an authorized squash merge, inspect the final squash author and the
  complete title/body. GitHub can generate co-author trailers from PR commits;
  supply a reviewed merge message that contains no assistant credits instead of
  accepting generated attribution. GitHub's own merge-service committer is
  platform metadata; it must not be confused with an assistant credit.
- After publishing or merging, verify the actual GitHub commit's author,
  committer, and complete message. Do not claim attribution is correct based
  only on local Git config or the author header. If it is wrong, report it and
  correct it within the user's authorized scope; do not rewrite published
  history or change branch protection without explicit user authorization.

## Git and GitHub workflow

Create a focused branch for each task. Preserve unrelated dirty changes. Use
Conventional Commits, commit coherent slices, and inspect the final diff.

Do not merge, squash-merge, or otherwise integrate a branch or pull request
unless the user explicitly requests the merge. Creating or approving a pull
request, passing checks, or repository conventions do not authorize a merge.
When the user explicitly requests a merge, follow their specified method; if
they do not specify one, use squash merge.

For cross-repository changes:

1. Create linked branches in affected repositories.
2. Define or preserve the shared contract.
3. Add compatibility support where needed.
4. Update all consumers.
5. Link the pull requests.
6. Verify the supported version combination.
7. Merge in dependency order only after the user explicitly requests the
   merge; otherwise leave the pull requests open.
8. Remove compatibility code only in a later cleanup change.

Cross-repository pull requests must describe scope, affected contracts,
consumers, version changes, compatibility, migration, verification, rollout,
and rollback.

## Verification and scope

For desktop UI/runtime features, use the reusable harness documented in
`docs/desktop-testing.md` after building the affected workspace binaries. Exercise
the actual feature with keyboard/mouse input and inspect screenshots; verify its
expected state/output rather than treating a recording as proof. Record a concise
MP4 demonstration, report which assertions passed, and link the video in the final
response. When handing off a live desktop for user testing, leave the harness
session running and provide its SSH tunnel, VNC port, and password-file path.
Stop sessions created only for automated checks. Use a fresh artifact directory
for each run. Nested software-rendered checks do not establish direct-TTY or
hardware-GPU behavior.

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
replacement, packaging, and old-binary removal into one unreviewable change.
