# Configuration

The canonical user configuration is:

    ~/.config/knave/config.toml

`knave-settings` is the typed API for reading and modifying that file. It is
not a second persistence system. Every setting has a schema, validation rules,
and an explicit default.

Configuration changes must:

1. parse and validate before writing;
2. write through a temporary file and atomically replace the target;
3. preserve user-defined values, bindings, and unknown legacy fields when the
   migration policy permits them; and
4. record or expose the schema version used for the file.

Legacy Villain configuration may be read by an explicitly scoped compatibility
adapter during migration. New code must not create new competing config files,
silently delete legacy data, or treat runtime state as persistent settings.

Schema migrations need a defined forward path, backup/recovery behavior, and a
rollback story before they become the default writer behavior.
