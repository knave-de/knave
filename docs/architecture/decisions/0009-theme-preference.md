# ADR 0009: Desktop theme preference

Applications follow the Settings portal for dark mode. The portal reported
`org.freedesktop.appearance` `color-scheme = 0` (no preference) because nothing
set it, so portal-aware applications defaulted to light. Knave needs one
user-facing preference that applications and, later, the shell and compositor
can share.

Knave owns the optional `[appearance] theme` setting with values
`system` (default), `dark` and `light`. `system` reports no preference; `dark`
and `light` force `color-scheme` 1 and 2. This change persists the preference
and its portal projection only; shell and compositor theme palettes are
follow-ups that consume the same typed projection.

The shell and widget gallery read `theme` from the canonical configuration at
startup and select the matching bundled toolkit stylesheet. `system` selects
dark CSS deterministically until Knave has an authoritative system color
preference source. The portal backend continues to report `system` as no
preference and reads the separate `[portal] color_scheme` fallback. The
pre-existing `[portal] color_scheme` key stays
readable and is still honored while `theme = "system"`, preserving existing
files; `dark`/`light` take precedence. The portal watches the configuration
directory and re-publishes `SettingChanged` when the file is saved, so editing
the file is the apply step and no portal restart is needed. `knavectl reload`
remains the compositor/wallpaper reload and is unrelated to this projection.

Adding the table is additive: a missing `[appearance]` defaults to `system`,
unknown keys are preserved, and no public desktop IPC change is required.
Shell surfaces and the standalone gallery use the same typed preference and
apply it to rebuilt toolkit scenes. Rollback removes the table; unknown source
keys remain. The portal consumes the new field from `knave-config`, so the
consuming repositories are built together at matching revisions.

Verification: `knave-config` unit tests cover the `system` default, write/load
round trips, unknown-key preservation and rejection of unsupported values; the
portal Settings tests cover the `theme` to `color-scheme` mapping. Live portal
and application behavior still needs a running-session check.
