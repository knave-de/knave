# Isolated development workflow

`scripts/knave-dev.py` owns coordinated development across Knave, Villain,
Knave Shell and the portal backend. It uses ordinary Git worktrees and Cargo;
there is no parent repository or agent-service dependency. Python 3.12+, Git,
Rust with rustfmt/Clippy, and each component's native build dependencies are
required. Nested tests also need the tools in [desktop testing](desktop-testing.md),
plus `dbus-daemon`, `dbus-update-activation-environment`, `gdbus`, and `pipewire`.

## Create and develop

Keep the four source repositories as siblings. From Knave:

```sh
dev=(python3 scripts/knave-dev.py)
"${dev[@]}" create overview-animations --change knaveshell --change villain
"${dev[@]}" status overview-animations
```

The default state root is `$XDG_DATA_HOME/knave-dev`, normally
`~/.local/share/knave-dev`. Override it with `--state /absolute/path` before the
subcommand. `create` prints the new workspace path. Edit and commit inside its
component worktrees. Changed components receive `dev/FEATURE` branches; all
others are detached at fixed commits. Existing source worktrees, branches and
uncommitted changes are preserved. Repeating an existing name fails.

By default each component starts at its source checkout's HEAD. Choose another
revision explicitly with repeatable `--rev REPO=REV` arguments, for example
`--rev xdg-desktop-portal-knave=origin/master`. Creation does not fetch, guess
compatible revisions, or copy uncommitted edits. The manifest records the exact
selected commits. If a previously unchanged component needs edits, create a new
workspace with that component selected rather than silently moving its pin.

Agents select affected repositories, implement features and commit coherent
changes. These commands provide deterministic infrastructure; they do not spawn
agents or decide ownership. Agents must follow each repository's `AGENTS.md`.

## Build, freeze and test

```sh
"${dev[@]}" build overview-animations --profile debug
bundle=$("${dev[@]}" bundle overview-animations)
"${dev[@]}" test "$bundle"
"${dev[@]}" verify "$bundle"
```

Build requires clean component worktrees, including no untracked source files.
It archives their commits into a build snapshot and generates Cargo patches
pointing every shared Knave crate to that snapshot. It resolves locks there,
checks dependency provenance, then runs formatting, all-target tests, strict
Clippy, and binary builds in each repository. Original Cargo manifests and
lockfiles remain unchanged. Failed builds retain their logs but cannot be
bundled. There is one account-wide build lock and one Cargo job by default;
`--jobs N` permits 1–8 jobs. Components are built sequentially.

Debug builds disable debug information to bound memory/disk usage. Release
builds use `--profile release`. Targets are cached per state root and component,
with the shared build lock protecting them across features;
build snapshots and evidence are retained until explicitly removed. There are
no background daemons or watchers. The build is tied to the host toolchain and
system libraries; it is not a portable or hermetic distribution package.

`bundle` freezes the latest build, only if successful, into a unique directory with
all six executables, desktop resources, pinned test tools, source/resolved
lockfile hashes, resolved lockfiles, toolchain, build options and a SHA-256 file
inventory. Files/directories are made read-only. A feature can have many bundles;
an earlier bundle is never overwritten. This prevents accidental mutation, not
deliberate modification by its owner. `verify` detects changed files or binaries.

`test` runs the bundled harness against these frozen executables. It verifies
Overview, launcher search, Kitty focus and typed input, workspace navigation,
and a finalized MP4. A private D-Bus with its own service directory activates
the bundled portal backend against a private PipeWire daemon; Settings.Read and the activated process executable
are checked. It never imports activation into the host bus or systemd manager.
`--display N` and `--vnc-port N` select unused nested-test resources. VNC defaults
to disabled for this command. Tests save logs, state assertions, screenshots and
video under `STATE/runs`; failures retain their temporary run and log paths.

Passing nested tests creates a verification record bound to the bundle manifest
hash. This is required for PR publication, export and TTY selection. It does not
establish DRM/KMS, hardware acceleration, physical input, full portal capture,
PipeWire streams, browser/OBS integration or multi-monitor behavior. Those
remain separate feature-specific checks and real TTY review. A recording alone
is never treated as proof.

## Publish coordinated changes

```sh
"${dev[@]}" pr overview-animations --bundle "$bundle" --base master
```

This explicit command pushes affected branches and creates draft PRs in their
original repositories, using authenticated `gh`. It requires the worktrees to
match the tested bundle's commits. It does not commit, rebase or merge anything.
PRs list all component revisions and the verification identity. Returned PR URLs
are stored in `workspace/pull-requests.json`; retries reuse open PRs. Agents in
T3 Code must additionally register each URL with `link_pull_request`.

For public shared-contract changes, update every affected consumer together.
Development Cargo patches are not published dependency pins: before integration,
commit the intended upstream Git revisions to consumer manifests and locks,
rebuild/retest the resulting coordinated set, and merge in dependency order only
when explicitly requested. A successful local patched build alone does not prove
an independently checked-out consumer builds against its published dependencies.

## Hand off to a dedicated TTY account

Export from the development account:

```sh
"${dev[@]}" export "$bundle" --output /chosen/shared/overview-animations.tar.gz
```

The archive contains the exact bundle, verification and recorded evidence. Give
the dedicated test account access to this archive and a checkout of Knave's
tooling; its home does not need access to the developer's private state directory.
Archives are trusted local handoffs, not signed release artifacts.

In the dedicated non-root test account, using default XDG config/data locations:

```sh
bundle=$(python3 /path/to/knave/scripts/knave-dev.py import /chosen/shared/overview-animations.tar.gz)
dev=(python3 "$bundle/tools/knave-dev.py")
"${dev[@]}" tty init --dedicated-test-account
"${dev[@]}" tty use "$bundle"
"${dev[@]}" tty status
```

Initialization explicitly marks this account for testing; it does not create a
Unix account, install binaries or change your normal desktop. Selection verifies
the bundle and its evidence and atomically replaces `STATE/current`. The previous
selection is retained for `tty rollback`. Selection is refused while a managed
TTY session is running or recovery is pending.

Log into that account on an active local TTY, then:

```sh
"${dev[@]}" tty start
# Or use a private normalized copy of a chosen configuration:
"${dev[@]}" tty start --config /path/to/config.toml
```

The launcher validates the logind user, active local seat and runtime bus. SSH
and inactive/remote seats are rejected. It resolves `current` once, holds the
session lock through teardown, creates private configuration and pins all
executable/resource paths to that bundle. It temporarily adds matching user-local
D-Bus activation, portal routing and a systemd unit. Both direct D-Bus Exec and
systemd ExecStart use the same pinned backend/configuration. Existing conflicting
portal metadata or running portal services cause an error instead of being
overwritten. The dedicated account must not run another graphical session.

Exit the session normally or send SIGTERM to the launcher. It stops the desktop
process group and portal services, removes its activation files and restores the
previous user-manager activation environment. It retains session logs/config.
It never installs into `/usr/local`, overwrites `~/.local/bin`, or edits the
canonical configuration. SIGKILL/power loss cannot guarantee cleanup; after an
interruption run `tty recover` in this account before selecting another bundle.
Externally edited activation files are preserved and reported.

After ending the session:

```sh
"${dev[@]}" tty rollback
"${dev[@]}" tty start
```

A new session is required for every bundle change. Bundles are host-compatible
builds, so transfer between accounts on the same machine is the intended path.

## Cleanup and regression checks

`remove FEATURE` removes only pristine workspaces still at their initial
commits; it refuses dirty worktrees or feature commits and retains branches.
For completed features with commits, use normal Git worktree cleanup after
reviewing their publication status. Bundles and evidence are retained separately.

Run the tooling regressions with:

```sh
python3 -m unittest discover -s scripts -p 'test_*.py' -v
```

These include actual Git worktree operations and tiny coordinated Cargo projects
built in debug and release, plus failure/rollback, dependency provenance, lockfile
preservation, bundle tampering, handoff and TTY gating tests. They do not acquire
the physical seat or modify the account's real portal services.
