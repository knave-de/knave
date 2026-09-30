# Configuration

The canonical user configuration is:

    ~/.config/knave/config.toml

`knave-config` is the typed API for reading and modifying that file. It is
not a second persistence system. Every setting has a schema, validation rules,
and an explicit default.

The top-level sections are owned by Knave:

- [compositor] is the typed projection consumed by Villain.
- [session] controls environment startup and process supervision.
- [shell] controls which Knave shell surfaces are started.

The session fields are:

- backend: auto, tty, or winit;
- compositor_binary and shell_binary: the supervised executables; and
- restart_on_failure: whether bounded component restart is enabled.

`start_bar` starts the persistent bar role. `start_overview_service` starts one
session-owned Overview process, hidden until toggled by Villain; it defaults to
true. In schema 2, setting it false disables Overview for that session. Schema
1 used false to select the on-demand process, so loading schema 1 projects this
setting as true and keeps Overview available under the session-owned model.
`knave config migrate` persists that conversion atomically. These settings are
consumed by the Knave session supervisor and are not a second shell config file.

Villain and the shell receive typed projections from Knave. They do not create
competing user-facing configuration files.

Configuration changes must:

1. parse and validate before writing;
2. write through a temporary file and atomically replace the target;
3. preserve user-defined values, bindings, and unknown fields when the
   preservation policy permits them; and
4. record or expose the schema version used for the file.

Villain root-level `modkey`, `environment_file`, `[input]`, and
`[[bind]]` values are read as a compatibility projection when no `[compositor]`
table exists. The original keys remain in the editable TOML document, so
loading or writing the file does not silently delete source data. New
configuration is written under `[compositor]`. Run `knave config migrate`
explicitly to materialize that projection in an existing file. The
command refuses to create a missing file, writes atomically, and leaves root-level
keys and unknown TOML values in place for rollback. `knave config check` remains
read-only.

Schema 1 to 2 migration preserves unknown TOML and source keys. It maps the
schema-1 on-demand Overview setting to the schema-2 session-owned service and
persists the typed projection atomically when `knave config migrate` runs.
Rollback is the previous Knave/Villain/Shell version combination; the original
setting remains represented by the migrated true value, and unknown source
values are preserved. Version domains are tracked independently for configuration,
the public desktop API, the shell/UI protocol, and the private compositor protocol.

## Master/stack width

The configuration accepts the additive `[compositor] master_percent` setting: an integer
from 10 through 90, default 50. The master receives this percentage of usable
workspace width, excluding reserved panels; the stack receives the remainder.
One tiled window still fills the usable area. Villain owns layout and input;
Knave only validates, preserves and writes the setting. Existing schema-1 files
need no special setting conversion, and unknown TOML entries remain preserved.

Villain's matching `feat/layout-split-resize` change consumes this projection.
Older Villain binaries ignore the new setting. Session and Shell behavior and
desktop API 1.1 are unchanged. Rebuilding Rust consumers that use complete
`CompositorConfig` literals requires initializing the new field. Deploy this
Knave configuration library before the matching Villain dependency pin.

Dragging the master/stack divider or using `resize-master` creates a workspace
session override; it never writes configuration. Reload updates workspaces
without overrides. `reset-master` clears the active override and uses the latest
configured default. Restart discards all overrides. To roll back, restore the
previous Villain binary and remove `resize-master`/`reset-master` entries from
custom bindings; the new percentage key may remain for a later upgrade.

## Window appearance

The additive `[compositor.appearance]` settings configure window gaps, borders,
corner radii, focus appearance, side shadows and background blur. See the
[complete contract and example](window-appearance.md). Existing files retain
the prior appearance, maximization disables all effects by default, and fullscreen
always bypasses them. No schema migration or desktop API change is required.
