# Window appearance

Knave owns the typed `[compositor.appearance]` contract in the canonical
`~/.config/knave/config.toml`. Villain owns geometry, rendering and hit testing.
Distances use integer logical pixels, opacity is an integer percentage, and
colors are `#RRGGBB` or `#RRGGBBAA` (straight RGBA). Omitted settings use the
defaults below. All settings reload through `knavectl reload`; invalid reloads
leave the entire running configuration unchanged.

## Geometry

| Table | Independent fields | Default | Valid range |
| --- | --- | --- | --- |
| `gaps.outer` | `top`, `right`, `bottom`, `left` | 6 | 0–4096 |
| `gaps.inner` | `top`, `right`, `bottom`, `left` | 6 | 0–4096 |
| `border.width` | `top`, `right`, `bottom`, `left` | 1 | 0–128 |
| `border.radius` | `top_left`, `top_right`, `bottom_right`, `bottom_left` | 3 | 0–4096 |

Gap values are per-edge contributions. Outer gaps inset the usable area after
panel reservations. Each tiled window contributes its inner gaps on every edge:
between two tiles the adjacent contributions add to 12 px. At the workspace
perimeter the 6 px outer and 6 px tile inner contributions add to 12 px.
Floating windows use outer gaps only, so their default clearance from the usable
area is 6 px. Border widths reserve additional space outside client
content, so increasing a border never covers application controls. One tiled
window uses the same gap policy. Floating windows keep their saved client size
and position, constrained inside the usable area, outer gaps and border allowance;
inner gaps only apply to tiles. Move/resize geometry remains client geometry.
For server-side decorated toplevels, the compositor titlebar sits inside this
same border frame; the border surrounds both the titlebar and client content.

Gap insets shrink proportionally when the configured opposing edges exceed the
available dimension, retaining at least one logical pixel of client content.
Adjacent corner radii shrink with one common scale when their sums exceed the
window width or height. Radii describe the outside border contour; the content
contour subtracts adjoining border widths. Popups retain their own geometry and
are not clipped to the parent window's rounded rectangle. Clicking a border
focuses its window without sending the click into the application; removed
rounded content corners do not receive window input. Shadows do not receive input.

## Focus appearance

The `focused` and `unfocused` tables each define:

| Field/table | Parameters | Default | Valid range |
| --- | --- | --- | --- |
| `opacity` | Window-content opacity | 100 | 0–100 |
| `border_color` | `top`, `right`, `bottom`, `left` colors | Theme-derived | RGBA color |
| `blur` | `enabled`, `radius`, `passes` | false, 8, 1 | radius 0–64; passes 1–4 |
| `shadows.top`, `.right`, `.bottom`, `.left` | Settings below | Theme-derived when omitted | Settings below |

A shadow side has `enabled` (false), `color` (`#00000060`), `blur_radius` (12,
0–128), `spread` (0, −128–128), `offset_x` (0, −256–256), and `offset_y` (0,
−256–256). Positive x moves right, positive y moves down. Each side paints its
nearest-edge sector outside the border contour; corner sectors meet diagonally.
Its Gaussian falloff stops after four blur radii. Side colors contain their own
opacity, independently of window-content opacity. Negative spread contracts the
shadow source. Disabled sides contribute nothing.

When no explicit focus border colors or focused shadow sides are configured,
the active edge, inactive neutral edge, and focused shadow follow the selected desktop
theme's color/light/dark and contrast policy. High contrast adds a 2 px
`--text` outline offset 1 px around the focused window when that view enables
borders, and disables the theme shadow. Explicit compositor edge colors and
shadow tables take precedence over their theme-derived equivalents.
Villain resolves these theme tokens when it loads configuration; run
`knavectl reload` after changing a theme CSS file.

Background blur samples only content below that window in stacking order, with
padding for the blur footprint. It excludes the window itself, cursor and upper
shell layers. `radius` is the finite sampling radius of each horizontal/vertical
Gaussian pair; `passes` repeats the pair and broadens the result. Blur is visible
through client transparency and compositor opacity. It does not blur application
text. Border/shadow colors have independent alpha; content opacity does not dim
them. Client-owned decorations and transparency remain application-controlled.
Keyboard focus selects the profile and triggers a repaint even without a new
client buffer. Taking focus into the shell selects unfocused styling for windows.

## Views

`views.tiled`, `views.floating`, and `views.maximized` each have independent
boolean switches: `gaps`, `borders`, `radius`, `shadows`, `opacity`, `blur`.
Tiled and floating switches default true (the underlying numeric defaults still
produce no effects). Every maximized switch defaults false. Enabling maximized
gaps uses outer gaps only, after panel reservations. Maximized borders reserve
space around content; switches for radius, shadows, opacity and blur are independent.
Unmaximizing restores normal tiled placement or saved floating geometry.

Fullscreen always uses the entire output, including space otherwise reserved by
panels, with no compositor gaps, borders, rounding, shadows, opacity multiplier
or blur. There is no fullscreen appearance switch. Entering fullscreen retains
saved maximization; exiting fullscreen restores its configured appearance.
Shell layer surfaces and unmanaged X11 surfaces do not acquire window effects.
Workspace previews and Overview panes use the same window appearance policy.

## Example

Add these tables to an existing file; do not duplicate existing table headers.
The example overrides the theme-derived colors and enables an unfocused shadow.
The equivalent unfocused shadow tables support every parameter too.

```toml
[compositor.appearance.gaps.outer]
top = 12
right = 16
bottom = 20
left = 16
[compositor.appearance.gaps.inner]
top = 4
right = 4
bottom = 4
left = 4
[compositor.appearance.border.width]
top = 2
right = 2
bottom = 3
left = 2
[compositor.appearance.border.radius]
top_left = 12
top_right = 12
bottom_right = 8
bottom_left = 8
[compositor.appearance.focused]
opacity = 100
[compositor.appearance.focused.border_color]
top = "#8aadffff"
right = "#8aadffff"
bottom = "#8aadffff"
left = "#8aadffff"
[compositor.appearance.focused.blur]
enabled = false
radius = 8
passes = 1
[compositor.appearance.focused.shadows.top]
enabled = false
color = "#00000040"
blur_radius = 8
spread = 0
offset_x = 0
offset_y = -2
[compositor.appearance.focused.shadows.right]
enabled = true
color = "#00000060"
blur_radius = 12
spread = 1
offset_x = 2
offset_y = 0
[compositor.appearance.focused.shadows.bottom]
enabled = true
color = "#00000090"
blur_radius = 16
spread = 2
offset_x = 0
offset_y = 4
[compositor.appearance.focused.shadows.left]
enabled = true
color = "#00000060"
blur_radius = 12
spread = 1
offset_x = -2
offset_y = 0
[compositor.appearance.unfocused]
opacity = 85
[compositor.appearance.unfocused.border_color]
top = "#505868ff"
right = "#505868ff"
bottom = "#505868ff"
left = "#505868ff"
[compositor.appearance.unfocused.blur]
enabled = true
radius = 8
passes = 1
[compositor.appearance.views.maximized]
gaps = false
borders = false
radius = false
shadows = false
opacity = false
blur = false
```

Run `knave config check`, then `knavectl reload`. Initial deployment needs a
matching Villain binary and a session restart; subsequent settings changes reload.
