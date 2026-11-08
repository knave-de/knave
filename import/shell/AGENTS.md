# Knave Shell Agent Instructions

Knave Shell is the Rust/wgpu desktop-facing shell for the independent Knave
Desktop Environment. It owns shell presentation and interaction surfaces while
Knave owns settings/session orchestration and Villain owns compositor policy.
The repository is Cargo-only; it has no Qt, CMake, C ABI, or host-desktop
integration runtime.

## Required workspace flow for all edits

All repository changes, including code, configuration, scripts, tests,
documentation and `AGENTS.md` edits, must use the managed
[development workspace flow](https://github.com/knave-de/knave/blob/master/docs/dev-workflow.md). Read-only investigation may
use the source checkouts; edits and commits belong in feature worktrees.
Creating a branch in a source checkout does not satisfy this requirement.

Before the first edit, create a workspace from the Knave checkout with
`./knave-dev create FEATURE --change REPO`, repeating `--change` for every
repository you intend to edit. An existing managed workspace may be reused
only when its manifest selects all affected repositories for changes. Check
`workspace.json`, the current branches and Git status before editing. If an
additional repository is needed, create a new workspace with that repository
selected; do not edit a detached dependency worktree or silently move its pin.
Preserve the source checkouts, their branches and unrelated work.

Edit, commit and run checks inside the selected feature worktrees. For runtime
or coordinated component changes, use the managed build, frozen bundle, nested
test and verification steps before handoff. For documentation-only edits,
review the diff and run `git diff --check`; a binary rebuild is unnecessary.
Keep existing verified bundles immutable and leave running user sessions alone.

## Startup and investigation

Before editing:

1. Read this file and the umbrella `knave/AGENTS.md` when working on a
   cross-repository change.
2. Read relevant README, architecture, API, and build documentation when it
   exists.
3. Inspect the complete owning module and search all of its consumers.
4. Check `git status` and preserve unrelated changes.
5. Classify whether the change affects UI, public desktop API, Wayland
   protocol, compositor IPC, configuration, build, packaging, or runtime behavior.
6. Write a short impact summary before changing a shared contract.

Do not implement a local-looking change until its cross-component effects have
been checked.

## Ownership and contracts

Knave owns the user-facing configuration and settings model. This repository
consumes Knave-owned settings only through typed APIs and public desktop
contracts. It must not introduce another user configuration source.

Villain remains the source of truth for workspace, window, focus, and preview
state. The shell consumes typed desktop contracts and must not duplicate
compositor policy.

Treat these as contracts:

- the Knave public desktop API payloads;
- shell roles and layer-shell behavior;
- workspace/window model fields;
- configuration and runtime environment variables.

Changing any of them requires consumer search, package-set analysis, tests, and
documentation updates where user behavior changes. The shared desktop contract
has no separate API version; update its consumers together.

## Build policy

Cargo is the only build system for this repository. Use the workspace profile
selected by the command and keep debug/release artifacts separate. The
installer always requires an explicit user, system, or custom prefix and must
not silently install into `/usr/local`.

After build changes, update `README.md` with only the commands and behavior
that actually exist.

## Code quality

Keep UI composition, rendering, Wayland integration, and desktop transport in
explicit crates. The standalone `knave-ui` repository owns the UI toolkit
and renderer; consume those crates through their pinned Git revision. Do not mix rendering, transport, configuration, and
compositor policy in one module.

Use typed errors and honest unavailable/disconnected states. Do not fabricate
empty successful data when Villain is unavailable. Preserve reconnect behavior.

Review lifecycle and ownership carefully, especially for layer surfaces, GPU
resources, desktop API clients, snapshot workers, and spawned processes.

Comments should explain invariants or non-obvious reasons. Do not narrate
obvious C++, Rust, or QML code. Keep comments short.

## Performance and resource usage

Review resource behavior before adding QML timers, file or socket watchers,
IPC subscriptions, image providers, background tasks, threads, caches, or
parallel work.

- Give each activity an explicit owner, lifetime, cancellation path, and
  cleanup path for reload, disconnect, and shutdown.
- Bound subscriptions, model updates, preview/image caches, queues, retries,
  and concurrency. Do not create one worker or watcher per event without a
  measured bound.
- Prefer event-driven notifications, debouncing, batching, and backoff over
  polling or continuously waking event handlers.
- Measure CPU, resident memory, threads, file descriptors, and wakeups under
  idle, normal, and stress workloads when a change can affect them.

Compilation and functional tests do not establish acceptable shell performance.

## Documentation

Keep README instructions short and executable. Add architecture or migration
documentation only when a boundary or public contract changes. Do not write
long documentation for routine refactors or repeat source code in prose.

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

## Git and GitHub

- Preserve unrelated dirty changes.
- Create a new focused branch for every task.
- Use Conventional Commits.
- Commit every coherent implementation slice.
- Link dependent cross-repository pull requests.
- Describe affected contracts, consumers, compatibility, verification,
  rollout, and rollback in cross-repository pull requests.
- Never merge, squash-merge, or otherwise integrate a branch or pull request
  unless the user explicitly requests the merge. Creating or approving a pull
  request, passing checks, or repository conventions do not authorize a merge.
- When the user explicitly requests a merge, follow their specified method; if
  they do not specify one, use squash merge.

## Verification

For desktop UI/runtime features, follow `../knave/docs/desktop-testing.md` and
the umbrella Knave desktop-feature verification instructions. Build the affected
workspace binaries, exercise the feature in the isolated Winit harness, verify
state/output and screenshots, and record an MP4 demonstration for the user.
Provide the video link and assertions in the final response. Leave a requested
interactive handoff running with its VNC/SSH connection details; stop sessions
used only for automated checks. This does not establish direct-TTY/GPU behavior.

For Rust changes, run:

```console
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

A successful compile does not prove layer-shell, focus restoration, GPU
behavior, direct-TTY startup, or installed-binary startup. Run the relevant
Wayland/GPU smoke tests separately and report direct-TTY coverage explicitly.

Before completion, inspect the final diff, run `git diff --check`, check
generated files, and report anything not tested.
