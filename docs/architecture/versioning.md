# Versioning and compatibility

These version domains are tracked independently:

- binary and library versions;
- Knave configuration schema;
- public desktop API;
- private compositor protocol; and
- shell/UI protocol.

Each public contract must state its version, supported compatibility range,
additive-change rules, breaking-change rules, and deprecation path. A `0.x`
version does not waive this requirement.

Breaking changes require consumer discovery, a compatibility adapter or
migration where practical, updated tests and documentation, and a rollback
plan. Removing a command, field, key, protocol message, or binary is a later
cleanup step after the replacement has been deployed and observed.

Cross-repository changes must identify the supported Knave/Villain version
combination and merge in dependency order. A library passing its own tests is
not evidence that the ecosystem remains compatible.

## Desktop API 1.1: maximization

API 1.1 adds `maximize-focused`, `unmaximize-focused`, and
`toggle-maximize-focused` dispatch actions and a default-false `maximized`
window-summary field. Fullscreen takes precedence over saved maximization;
unmaximizing does not exit fullscreen or restore a minimized window.
Existing commands retain their behavior. No configuration schema change occurs.

API 1.0 clients may keep using existing actions with a 1.1 server and ignore
unknown summary fields. A 1.1 client can read 1.0 summaries; new commands require
a 1.1 server. Query `version` before offering them to mixed-version deployments;
there is no automatic negotiation or downgrade. Rust consumers rebuilding with
the updated crate must initialize `maximized` in summary literals and handle
new command variants. Existing pinned Shell binaries need no update.

Deploy the Knave API/client commit before the matching Villain dependency pin.
Rollback both binaries together to remove new commands; no persisted window
state or configuration migration needs rollback. The independent library and
binary package versions remain 0.1.0 during this unreleased development change.

API 1.2 adds [desktop state subscriptions](decisions/0002-desktop-state-subscriptions.md).
