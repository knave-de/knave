# ADR 0010: Unversioned configuration before release

Knave has no released configuration format to support. Use the current typed
TOML structure directly, without `schema_version`, version validation, or
`knave config migrate`. Remove the v1 Overview conversion and the root-level
Villain compositor projection. `start_overview_service = false` always disables
Overview; compositor settings belong under `[compositor]`.

Knave owns this format. Update CLI/session diagnostics, development and desktop
test fixtures, and Villain/portal examples together. Package versions and
external protocol versions are unchanged. Typed defaults, validation, atomic
writes, and preservation of unknown TOML during settings writes remain.

No migration or compatibility layer is provided for unreleased formats. Local
configs must use the current sections; the obsolete version line can be deleted.
To undo this change, restore the preceding coordinated source revisions.
Verification covers config defaults, validation and writes, Overview disablement,
CLI commands, development scripts, and consumers built against this checkout.
