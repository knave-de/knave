# Villain configuration boundary

The canonical user configuration is
`~/.config/knave/config.toml`, owned by Knave settings.

Villain consumes the typed [compositor] projection:

- modkey and bind define compositor key dispatch;
- input defines global touchpad policy;
- device lists per-device rules matched by name; and
- environment_file selects an optional environment fragment relative to the
  Knave configuration file.

Villain applies the global [compositor.input] policy, then every matching
[[compositor.device]] rule in file order, when a libinput device is added and
again on `knavectl reload`. A device is enabled unless a matching rule sets
`enabled = false`, so removing that rule and reloading restores events. Only the
fields present in a rule are changed; the rest keep their libinput values. The
`devices` desktop query reports each device, its current enabled state, its
libinput capabilities, and the index of the last rule that matched it.

Compositor settings are read only from `[compositor]` and its nested tables.
There is no root-level compatibility projection or configuration schema version.

Runtime state, socket paths, seat/VT discovery, and process bookkeeping belong
to the runtime/session boundary rather than persistent user configuration.
Source data must not be silently deleted.

The tracked `config.example.toml` demonstrates the `[compositor]` section of
Knave's canonical configuration. Villain does not install a separate config file.
