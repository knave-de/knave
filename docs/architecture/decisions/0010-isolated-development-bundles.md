# ADR 0010: Isolated development bundles

## Decision

Knave owns a Python development CLI alongside the existing desktop harness.
The five repositories remain independent, including the standalone Knave UI
library repository. Each feature has matching Git
worktrees, explicit changed components and detached fixed dependencies.

Builds archive clean commits, patch shared Cargo dependencies to one coordinated
source snapshot, resolve snapshot-only lockfiles, and verify every component.
One account-wide build lock and bounded Cargo jobs limit resource contention.
Completed binaries/resources are frozen before nested testing. Verification
records bind assertions and evidence to a hashed bundle manifest; the same
artifacts are exported to a dedicated account for human direct-TTY testing.

TTY selection uses an atomic symlink under a session lock. A local logind login
is required. Activation metadata and user units use absolute pinned paths for
the entire session. Existing regular portal metadata and permissions are saved
in the recovery journal before temporary replacement; active portal services are
stopped and restored with their original files and environment after the test.
SSH and terminal logins can stay open, while another graphical login under the
same user is refused. Metadata symlinks are refused. Interrupted sessions retain
the recovery journal. The normal user's desktop and canonical config are outside
this path.

Feature configuration lives in `STATE/workspaces/FEATURE/config/`, seeded once
from `STATE/config/`. The launcher generates a runtime copy with pinned lifecycle
settings. Explicit reload validates and synchronizes that copy before dispatching
the compositor reload; rejected changes restore previous runtime files. Config
sources and optional sibling environment files stay editable across sessions and
workspace cleanup. Startup changes require a new session. The existing runtime
configuration and reload protocol are unchanged.

## Impact and boundaries

Ownership is limited to build/test tooling and harness integration. Runtime
contracts, package versions and configuration schema are unchanged. Villain,
Knave UI, Shell and the portal backend consume the same shared Knave snapshot during
verification; published dependency revisions still require explicit coordinated
updates before integration. No compatibility negotiation is added.

Nested checks now include private portal activation and Settings, alongside the
existing interaction/state/video tests. They do not establish full capture,
hardware acceleration, DRM/KMS, physical devices or installed-login behavior.
Direct-TTY review remains a human responsibility. Build/test caches and evidence
have explicit filesystem lifetimes; there is no daemon or polling scheduler.

## Rollout, verification and rollback

The CLI is opt-in; existing Cargo/install/harness commands continue to work.
Regression tests exercise real worktrees and small debug/release coordinated
builds, including local dependency selection and untouched source lockfiles.
Actual coordinated bundles are tested through nested Winit and private D-Bus.
TTY guards, activation paths and recovery are tested without changing host units;
physical-seat behavior must be reviewed in the dedicated account.

Rollback selects the previous verified bundle after the current session ends.
Removing the tooling does not migrate or delete user configuration. Detailed
commands and coverage are in [the development workflow](../../dev-workflow.md).
