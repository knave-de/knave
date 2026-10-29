# ADR 0009: Desktop theme preference

Knave owns the desktop appearance selection in `[appearance.theme]`: a named
`theme`, a `mode` of `system`, `dark`, or `light`, a `contrast` of `standard`
or `high`, and a `chroma` of `full-color` or `monochrome`. The portal projects
only the mode to `org.freedesktop.appearance` and uses its existing
`color_scheme` fallback for `system`. Knave UI resolves `system` to dark until
a system color preference source exists. Only the table form is supported;
unknown fields are preserved.

The two accessibility preferences are orthogonal to the light/dark scheme.
High contrast strengthens text, border, and focus distinctions. Monochrome
applies a grayscale palette to UI chrome; images, color swatches, and explicitly
colored rich text remain content. The Knave UI policy layer overrides named
theme palette tokens, while application CSS remains last and can override it.

The Linux standalone app host loads named CSS from
`$XDG_DATA_HOME/knave/themes/<theme>/{dark,light}.css`. Its event-driven watcher
tracks the config and selected file, including atomic replacement. The toolkit
validates and applies the scheme, accessibility palette, desktop CSS, named CSS
and app CSS together;
invalid edits preserve the last valid appearance. Missing CSS uses the bundled
palette. Shell surfaces continue to select their bundled palette at startup;
they do not yet consume named CSS. No desktop IPC change is needed.

The Knave configuration, app host, shell and portal must ship with compatible
configuration revisions. To roll back, revert them together and restore the
previous table configuration. Tests cover parsing and named round trips,
validation, CSS layering, and failed style replacement; live verification must
exercise reload and recovery in a running app.
