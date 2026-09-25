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

`start_bar` starts the persistent bar role. `start_overview_service` is an
explicit opt-in for an always-open overview; it defaults to false because the
normal overview path is an on-demand `knave-shell overview` process launched by
Villain. These settings are consumed by the Knave session supervisor and are
not a second shell config file.

Villain and the shell receive typed projections from Knave. They do not create
competing user-facing configuration files.

Configuration changes must:

1. parse and validate before writing;
2. write through a temporary file and atomically replace the target;
3. preserve user-defined values, bindings, and unknown legacy fields when the
   migration policy permits them; and
4. record or expose the schema version used for the file.

Legacy Villain root-level `modkey`, `environment_file`, `[input]`, and
`[[bind]]` values are read as a compatibility projection when no `[compositor]`
table exists. The original keys remain in the editable TOML document, so
loading or rewriting the file does not silently delete legacy data. New
configuration is written under `[compositor]`. Run `knave config migrate`
explicitly to materialize that projection in an existing legacy file. The
command refuses to create a missing file, writes atomically, and leaves legacy
keys and unknown TOML values in place for rollback. `knave config check` remains
read-only.

Schema migrations need a defined forward path, backup/recovery behavior, and a
rollback story. Version domains are tracked independently for configuration,
the public desktop API, the shell/UI protocol, and the private compositor protocol.
