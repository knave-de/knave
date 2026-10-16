# Package versions

Cargo package and binary versions identify Knave, Villain, Shell, and portal
backend builds. The initial package versions remain unchanged during this
unreleased development period.

The Knave desktop contract has no separate API version or negotiation. All
repositories that send or receive its messages update together. The `version`
commands report package identity and package version.

The configuration schema remains versioned because it describes persistent
user data and its migrations. Wayland interface versions are defined and
negotiated by their respective protocols.
