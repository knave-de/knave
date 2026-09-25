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
