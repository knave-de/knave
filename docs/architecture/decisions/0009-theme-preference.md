# ADR 0009: Desktop theme preference

Knave owns the desktop appearance selection in `[appearance.theme]`: a named
`theme` and a `mode` of `system`, `dark`, or `light`. The portal projects the
mode to `org.freedesktop.appearance` and uses its existing `color_scheme`
fallback for `system`. Knave UI resolves `system` to dark until a system color
preference source exists. Legacy string preferences remain readable and are
rewritten as a table by settings writes; unknown fields are preserved.

The Linux standalone app host opts into named CSS from
`$XDG_DATA_HOME/knave/themes/<theme>/{dark,light}.css`. Its event-driven watcher
tracks the config and selected file, including atomic replacement. The toolkit
validates and applies the bundled palette, desktop CSS and app CSS together;
invalid edits preserve the last valid appearance. Missing CSS uses the bundled
palette. Shell surfaces continue to select their bundled palette at startup;
they do not yet consume named CSS. No desktop IPC change is needed.

The Knave configuration, app host, shell and portal must ship with compatible
configuration revisions. To roll back, revert them together and restore the old
string configuration form. Tests cover parsing, legacy and named round trips,
validation, CSS layering, and failed style replacement; live verification must
exercise reload and recovery in a running app.
