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
- compositor_binary and shell_binary: the supervised executables;
- restart_on_failure: whether bounded component restart is enabled; and
- `[[session.startup]]` entries: optional commands launched after the shell
  roles have been started.

Each startup entry has a `command` array whose first string is the executable
and remaining strings are its arguments. Knave does not run a shell or expand
variables and globs. To use shell syntax, configure an explicit shell such as
`command = ["sh", "-lc", "command"]`. Up to 64 commands may be configured;
an empty executable or NUL character is invalid. Startup commands inherit the
session environment and receive its `WAYLAND_DISPLAY`; inherited `DISPLAY` and
`WAYLAND_SOCKET` values are cleared so nested sessions cannot target the host
desktop. Once Villain reports its XWayland display, Knave also sets `DISPLAY`
for startup commands, so X11 applications such as `xterm` can use the session's
XWayland server.

`start_bar` starts the persistent bar role. `start_overview_service` starts one
session-owned Overview process, hidden until toggled by Villain; it defaults to
true. Setting it false disables Overview for that session. These settings are
consumed by the Knave session supervisor and are not a second shell config file.

Villain and the shell receive typed projections from Knave. They do not create
competing user-facing configuration files.

Each `[[compositor.bind]]` entry overrides the default binding with the same key
combination; unmentioned defaults stay active, and entries that match no default
are additions. Omitting the list uses Villain's defaults. Overriding
`keys = "MOD"` replaces the default modifier-release Overview toggle. Settings
writes preserve custom bindings.

## Output scaling

`[compositor] scale` sets the global logical-to-buffer scale from 1.0 through
4.0, defaulting to 1.0. Villain rounds the value to the nearest 1/120th for
Wayland fractional-scale negotiation. `knavectl reload` applies the new scale to
the current output and sends updated preferred scales to connected clients. The
shell renders scaled buffers while keeping layout and pointer coordinates in
logical pixels. Clients without fractional-scale and viewporter support keep
using the integer `wl_output.scale` fallback. Per-output scale selection is not
part of this setting.

Configuration changes must:

1. parse and validate before writing;
2. write through a temporary file and atomically replace the target;
3. preserve user-defined values, bindings, and unknown fields when the
   preservation policy permits them.

The project is unreleased and uses one current configuration format without a
schema version or migrations. Compositor settings belong under `[compositor]`;
root-level compositor keys are not interpreted. `start_overview_service = false`
always disables Overview. See [the decision](decisions/0010-unversioned-configuration.md).

## Master/stack width

The configuration accepts the additive `[compositor] master_percent` setting: an integer
from 10 through 90, default 50. The master receives this percentage of usable
workspace width, excluding reserved panels; the stack receives the remainder.
One tiled window still fills the usable area. Villain owns layout and input;
Knave only validates, preserves and writes the setting. Unknown TOML entries
remain preserved.

Villain's matching `feat/layout-split-resize` change consumes this projection.
The session and Shell behavior and desktop message shapes are unchanged.
Rebuilding Rust consumers that use complete
`CompositorConfig` literals requires initializing the new field. Deploy this
Knave configuration library before the matching Villain dependency pin.

Dragging the master/stack divider or using `resize-master` creates a workspace
session override; it never writes configuration. Reload updates workspaces
without overrides. `reset-master` clears the active override and uses the latest
configured default. Restart discards all overrides. To roll back, restore the
previous Villain binary and remove `resize-master`/`reset-master` entries from
custom bindings; the new percentage key may remain for a later upgrade.

## Focus switching

The additive `[compositor] focus_cycle_order` setting selects the order
`focus-next`/`focus-previous` shortcuts walk a workspace's windows: `layout`
(master, then stack top to bottom) or `mru` (most recently focused first).
Default `layout`; both directions wrap. The
`[compositor.input]` table gains `focus_follows_mouse` (default true): when
true, moving the pointer onto a window switches keyboard focus; when false,
window focus changes on click only. Both keys are consumed by Villain's
`feat/focus-switching` change; consumers using complete `CompositorConfig`
literals need the new fields, and the Knave configuration library must deploy
before the matching Villain dependency pin.

## Window rules

`[compositor.window_rules]` configures initial placement for new windows. The
optional `apply = "on_open"` is the default. `[[compositor.window_rules.application]]`
entries match an exact Wayland `app_id`; `[[compositor.window_rules.rule]]`
entries match specific windows by any combination of exact `app_id`,
`title_contains`, and Rust-regex `title_regex`. Conditions within one rule are
ANDed. Application preferences apply first, then matching specific rules in
file order; later values override only fields they set. Supported actions are
`placement = "tiled" | "floating"`, `center`, `stay_on_top`, and
`workspace = 1..10`, limited to the configured `[compositor.workspace].predefined`
range. A rule cannot target a dynamic workspace; increase `predefined` to retain
the target slot. Centering implies floating when requested. Rules run once when
a window first opens, so later manual moves are respected. Invalid regexes,
empty matchers, and out-of-range or unretained workspace targets reject the full
config and leave the active configuration unchanged. The compositor compiles
regexes on config load.

The configuration is typed by `knave-config` and consumed directly by Villain;
Settings can read and write these same fields without a second persistence
format. The first release supports numbered workspace slots only.

## Input devices

The additive `[compositor.input]` table sets the global touchpad policy:
`tap_to_click`, `natural_scroll`, and `focus_follows_mouse`, all defaulting
to true. Per-device
overrides use one or more `[[compositor.device]]` rules:

```toml
[compositor.input]
tap_to_click = true
natural_scroll = true
focus_follows_mouse = true

[[compositor.device]]
name = "Example Touchpad"
tap_to_click = true
natural_scroll = false
sensitivity = 0.4
disable_while_typing = true

[[compositor.device]]
name = "*Receiver Mouse"
enabled = false
```

`name` is matched case-insensitively against the runtime device name and
supports the `*` and `?` wildcards. Every field except `name` is optional: an
unset field leaves the underlying libinput value untouched, so a rule that only
sets `sensitivity` never re-enables or re-configures tap-to-click. When several
rules match one device they apply in file order and later rules win. The global
`[compositor.input]` values apply first, then matching device rules override
them.

The supported fields are:

- `enabled`: when false, libinput stops delivering events from the device
  without detaching it; true restores them;
- `tap_to_click`, `tap_and_drag`, and `drag_lock`: touchpad tap behavior;
- `natural_scroll`: per-device scroll direction;
- `disable_while_typing`: disable the device while an internal keyboard types;
- `middle_emulation` and `left_handed`: button behavior;
- `sensitivity`: pointer acceleration speed in -1.0..=1.0, where 0.0 is the
  device default;
- `accel_profile`: `adaptive` or `flat`;
- `scroll_method`: `no_scroll`, `two_finger`, `edge`, or `on_button_down`;
- `scroll_button`: the button code used by `on_button_down`;
- `click_method`: `button_areas` or `clickfinger`.

Villain applies these rules when a device is added and re-applies them on
`knavectl reload`, so editing a rule and reloading enables, disables, or
re-tunes the device in the running session. `knavectl devices` lists the detected devices, whether each is currently
enabled, its libinput capabilities, and which rule index matched.
Device names are not written to configuration; use `knavectl devices` to
discover the exact name of the hardware you want to match. Writes are typed, so
unknown keys inside a rewritten `[[compositor.device]]` rule are dropped when
settings writes persist the projection, matching the existing
`[[compositor.bind]]` behavior; unknown keys elsewhere are preserved.

## Window appearance

The additive `[compositor.appearance]` settings configure window gaps, borders,
corner radii, focus appearance, side shadows and background blur. See the
[complete contract and example](window-appearance.md). Existing files retain
the prior appearance, maximization disables all effects by default, and fullscreen
always bypasses them. No desktop API change is required.

## Wallpaper

```toml
[shell.wallpaper]
image = "/absolute/path/to/wallpaper.png"
fit = "cover"
```

`cover` fills the screen with a centered crop; `contain` shows the full image
with black bars; `stretch` fills the screen and may distort it. PNG is supported;
convert other formats first. Omit `image` to return to black. Wallpaper
changes — a new assignment, a cleared one, and image or fit changes — apply
on `knavectl reload` without a session restart; the session spawns or stops
the wallpaper process as part of the assignment. See
[ownership and limits](decisions/0008-static-wallpaper.md).

## Desktop theme

```toml
[appearance.theme]
theme = "knave"
mode = "system" # system, dark, or light
```

Knave UI applications using the standard host load CSS from
`$XDG_DATA_HOME/knave/themes/<theme>/<mode>.css`; when `XDG_DATA_HOME` is
unset or relative it defaults to `~/.local/share`. Theme names contain only
ASCII letters, digits, `_` and `-`. `system` currently selects `dark.css`;
there is no authoritative system color preference yet. The bundled Knave
palette remains the base, desktop CSS overrides it, and app CSS follows it.
Missing CSS uses the bundled palette. Invalid configuration or CSS keeps the
last valid appearance and reports an error. Saving the selected CSS or config
updates running themed apps, including atomic file replacements.

Only the `[appearance.theme]` table form is supported. A scalar theme value
under `[appearance]` is invalid. The shell continues to select its
bundled palette at startup; named CSS and live updates for shell surfaces are
future work. The portal publishes `mode` as `color-scheme` (0/1/2) and still
uses `[portal] color_scheme` when `mode` is `system`.

## Workspace gestures

`[compositor.gesture]` contains optional `[[compositor.gesture.bind]]` entries
with `fingers` (integer >= 1), `direction = "horizontal"`, and
`action = "workspace"` or `"none"`. Entries override matching fingers+direction
only, in order; later duplicates win. Omitting an entry retains Villain's
three-finger horizontal workspace default. `none` passes that gesture to clients.
Unknown source fields survive settings writes; unknown enum values reject the
configuration. A successful reload cancels any active swipe before replacing
bindings; failed validation leaves the running configuration intact.
These are Villain actions, not knavectl commands. See the
[gesture decision](decisions/0010-workspace-swipes.md).

## Workspace cycling

```toml
[compositor.workspace]
predefined = 1
skip_empty = true
```

`predefined` defaults to 1 and must be positive. Those numbered workspaces are
permanent. Villain keeps at least one dynamic slot after the predefined prefix
and one empty slot after the highest occupied workspace. Dynamic slots grow
when selected and are trimmed only from the end; gaps stay in place. Forward
navigation grows at the end and does not wrap, while backward navigation stops
at workspace 1. The optional `skip_empty` boolean defaults to false. When
enabled, next/previous shortcuts and workspace swipes scan for occupied
workspaces first, including minimized windows. If forward navigation finds no
occupied workspace ahead, it selects the next blank slot. Explicit workspace
selection still reaches empty slots. Reload changes both settings atomically;
invalid values retain the running configuration.

Swipe previews keep their target for the current travel direction. New windows
in an intervening slot do not change an in-progress preview. Reversal selects a
new target in the other direction. If the previewed workspace becomes empty,
the gesture is cancelled and its remaining events are consumed.
See [the decision](decisions/0011-skip-empty-workspaces.md).
