# Villain WM Agent Instructions

Villain is the Wayland compositor and window manager for the Knave Desktop
Environment. It owns compositor mechanism and window-management policy:
outputs, rendering, input, workspaces, layout, focus, activation, XWayland,
and layer-shell handling.

Knave owns the user-facing configuration, settings, session lifecycle, public
desktop API, and cross-component compatibility policy. Preserve that boundary.

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
2. Read the relevant README, architecture, protocol, and build documentation.
3. Inspect the complete owning module and search all consumers of changed state,
   protocol messages, configuration keys, or commands.
4. Check `git status` and preserve unrelated dirty changes.
5. Classify the change as private, additive, behavioral, configuration,
   migratory, protocol, or breaking.
6. Write a short impact summary before changing a shared contract.

Do not attribute input, focus, or client behavior to a shell or application
until the compositor event path has been inspected.

## Contracts and compatibility

The desktop contract is owned by Knave's knave-desktop-api crate. knavectl is
the public control client.

Review and test changes to:

- the pinned Knave package revision and shared desktop messages;
- socket naming, permissions, and readiness behavior;
- workspace and window IDs and fields;
- focus and activation semantics;
- keybinding dispatch;
- configuration projection and reload;
- layer-shell lifecycle;
- nested/direct backend selection;
- environment and portal activation.

For each shared change, find every sender and receiver, update them together,
run coordinated package tests, and document user-visible effects.

Villain must not create a second user-facing configuration store. It consumes
validated projections from `[compositor]` in ~/.config/knave/config.toml.
The unreleased config format has no schema version or legacy migration paths.

## Runtime safety

Keep nested and direct-TTY behavior distinct:

- nested mode must not replace the host compositor's activation environment;
- direct mode requires a real VT, seat, GPU, and input session;
- inherited `DISPLAY` or `WAYLAND_DISPLAY` must not override real VT detection;
- compositor-intercepted keys must not be forwarded as stuck client modifiers;
- explicit focus transitions take precedence over stale pointer hit-testing;
- layer-shell surfaces remain separate from ordinary workspace windows.

Do not claim DRM/KMS, GPU, VT, portal, XWayland, or live focus behavior from
unit tests or compilation alone.

Runtime code must have explicit ownership, cleanup, permissions, process
reaping, shutdown, stale-socket handling, and typed errors. Avoid panics for
recoverable configuration, IPC, device, or process failures.

## Code quality and comments

Keep compositor mechanism separate from window-management policy. Keep IPC
serialization separate from state mutation. Keep focus state separate from
pointer delivery and layer-shell focus.

Do not perform opportunistic refactors or unrelated cleanup. Review error paths,
event ordering, modifier state, workspace transitions, client destruction,
XWayland stacking, output changes, and frame pacing for every relevant change.

Comments should explain invariants, protocol sequencing, safety assumptions, or
non-obvious reasons. Do not restate obvious Rust code or write long comments.

## Build and verification

For desktop runtime/UI features, follow `../knave/docs/desktop-testing.md` and
the umbrella Knave desktop-feature verification instructions. Build the affected
workspace binaries, exercise the feature in the isolated Winit harness, verify
state/output and screenshots, and record an MP4 demonstration for the user.
Provide the video link and assertions in the final response. Leave a requested
interactive handoff running with its VNC/SSH connection details; stop sessions
used only for automated checks. This does not establish direct-TTY/GPU behavior.

Villain is a Cargo workspace. Use:

```console
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Use isolated runtime/config directories for protocol tests. Run ignored
Wayland, layer-shell, XWayland, and frame-callback tests when the change
affects those paths.

Update README and examples when build, run, configuration, protocol, or
behavioral contracts change. Keep documentation concise and do not document
commands or behavior that the code does not provide.

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

Before completion, inspect the final diff, run `git diff --check`, check
generated files, and clearly report live behavior that remains unverified.

## Performance and resource usage

Review resource behavior before adding compositor watchers, timers, IPC
subscriptions, event-loop work, background tasks, threads, caches, buffers, or
parallel work.

- Keep the compositor event loop event-driven. Avoid busy loops, high-frequency
  polling, duplicate subscriptions, and retry storms.
- Bound per-client and per-surface work, queues, retries, memory, and
  concurrency. Do not create unbounded workers for clients, outputs, or events.
- Give every activity explicit cancellation and cleanup on disconnect, reload,
  client destruction, backend failure, and shutdown.
- Measure CPU, resident memory, threads, file descriptors, wakeups, and
  relevant latency under idle, normal, and stress workloads.

Compilation, unit tests, and a successful compositor start do not establish
acceptable resource behavior.
