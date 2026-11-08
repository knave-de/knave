# Shell configuration boundary

The canonical configuration is
`~/.config/knave/config.toml`, owned and written through Knave settings.

Shell code consumes typed configuration projections. It must not create a
competing shell configuration file, parse Villain's root-level file, or persist
runtime state as user settings. Knave owns migration and supplies the
compositor/session/shell projections through typed APIs.

The shell reads the `appearance.theme.mode` value from that same canonical file
when each surface process starts. `dark` and `light` select the corresponding embedded
toolkit stylesheet. `system` uses dark CSS deterministically until Knave has an
authoritative system color preference source. Theme changes take effect after
restarting the shell; reconstructed status-bar and overview scenes retain the
startup selection. This does not change the Settings portal's `color-scheme`
projection.

Adding a shell setting requires a schema entry, validation/default behavior,
serialization and migration review in Knave, and tests for unknown and source
data preservation. The shell must behave safely when the desktop API is
unavailable, incompatible, or missing an optional projection.
