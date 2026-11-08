# Window appearance implementation

Knave owns `[compositor.appearance]` in its canonical config. Villain consumes
`knave-config` at `3ef4a82a2dd173999f0500515f69b79fc9c32b88`. The configuration
revision is published on Knave master. Locked builds resolve the merged revision
without a machine-specific Cargo patch.

Configure `gaps.outer` and `gaps.inner` using top/right/bottom/left logical pixels.
Adjacent tiles add their inner-edge contributions. Outer gaps follow panel
reservations. Borders reserve space outside client content, with independent
`border.width` edges and `border.radius` corners. Floating client rectangles are
saved independently of effects and constrained to gaps/border bounds when those
geometry effects are configured; moves/resizes use the same projection.

`focused` and `unfocused` each have integer `opacity` (0–100), four
`border_color` edges, `blur` (`enabled`, radius 0–64, passes 1–4), and
`shadows.top/right/bottom/left`. Each shadow independently chooses enabled,
RGBA color, blur radius, signed spread and signed x/y offsets. Shadow nearest-edge
sectors meet diagonally outside the border contour. Background blur samples lower
stacking content with padding for its kernel, excluding the window, upper layers
and cursor. Popups remain outside the rounded parent mask. Client-owned chrome
remains application-controlled.

`views.tiled`, `views.floating`, and `views.maximized` gate gaps, borders, radius,
shadows, opacity and blur independently. Tiled/floating switches default true;
maximized switches default false. Numeric/effect defaults preserve prior visuals.
Fullscreen always uses the entire output and bypasses every compositor effect.
Top-layer shell surfaces such as the status bar are hidden and excluded from
input while fullscreen is active; overlay-layer surfaces remain available.
There is no configurable override. Saved maximization and floating placement
survive view transitions. Knave's `docs/architecture/window-appearance.md`
contains the complete validated contract, defaults, bounds and TOML example.

## Server titlebars

Villain provides a 34 logical-pixel titlebar when an xdg-toplevel uses the
`xdg-decoration` protocol in server-side mode. Villain initially offers server
side decoration and follows the client's negotiated mode; a client that
requests client-side decoration keeps its own chrome and receives no Villain
titlebar. Destroying the decoration object returns the toplevel to client-side
mode at its next surface commit. The titlebar is outside the client's
configured content rectangle and is not shown for fullscreen windows. XWayland
windows do not use this xdg-toplevel
negotiation and do not receive these titlebars.

The titlebar controls from right to left are close, maximize/restore, toggle
floating, and minimize. Double-clicking an empty titlebar toggles maximize;
dragging an empty titlebar moves the window. These controls are compositor
behavior and are not currently configurable. They are included in normal scene
rendering and workspace previews.

Reload validates before replacing config, cancels grabs, drops cached scenes and
recomputes geometry. Keyboard focus requests repaint without requiring a client
commit. Both nested and direct backends, previews and Overview panes share one
GLES effects pipeline. Layer surfaces and unmanaged X11 surfaces keep their policy.
Border clicks focus without forwarding input; clipped content corners reject input.
Tiled, maximized and fullscreen root/subsurface content is cropped to its assigned
client rectangle, including while a resize is pending or a client retains an
oversized buffer. Pointer hit testing uses the same rectangle. Popups keep their
own bounds; ordinary floating windows retain their client geometry. With effects
disabled, Smithay crop elements avoid allocating offscreen textures.

Disabled effects retain imported surfaces and Smithay incremental damage.
Enabled effects cache completed scenes using source identity, damage commit,
source viewport/transform, geometry, focus and configuration. One LRU bounds
cached storage to ten entries/36 Mi pixels. Temporary targets are independently
bounded to 36 Mi pixels and lower content is flattened when retained effect
textures would exceed the scene pixel budget. GPU buffers drop on replacement,
eviction, window destruction, reload and compositor shutdown. No worker, watcher,
subscription or timer is added. Render errors propagate as typed failures.
Changing window content can still require full offscreen composition; blur costs
more than ordinary rendering. Cache bounds do not include client buffers or driver
allocations. Direct-TTY and long-running GPU performance require separate checks.

Roll out the Knave config commit first, then this matching Villain build; restart
the compositor once. Subsequent edits use `knave config check` and `knavectl reload`.
Rollback restores the old Villain binary first; appearance tables may remain as
unknown data preserved by Knave. Schema 2 and package versions remain unchanged.
No config data or bindings are deleted.

## Verification on 2026-09-30

Knave's 23 tests and all 34 Villain tests passed with the matching git pins,
including isolated Wayland, layer-shell, XWayland and surfaceless GLES cases.
Format, warnings-denied Clippy, locked debug/release builds and diff checks passed.
Coverage includes independent gap/border geometry, tiny outputs, radius normalization,
border focus, clipped input, gap divider hit testing, opt-in maximized geometry,
fullscreen precedence, shader pixels, color alpha, directional shadows and finite
background blur. The Knave example and full documentation TOML are parsed in tests.

An isolated release Winit session with two Kitty Wayland clients verified tiled
appearance, focus-color changes, undecorated maximization/restoration, valid reload,
invalid-opacity rejection, PNG preview capture and 100 maximize toggles. Captures
were visually inspected. Shutdown removed the private desktop socket. Both actual
output rendering and PNG previews used the effects pipeline; no installed binary
or personal configuration was modified.

Release comparison used baseline `993a1f4` and the feature with identical clients,
cursor blinking disabled. In a settled three-second baseline sample, Villain used
zero additional CPU ticks, 154648 KiB RSS, eight threads, 42 FDs, one XWayland child
and two application children, with three main-thread context switches. The final
feature sample with asymmetric borders/radii, focus opacity, right/bottom shadows
and unfocused background blur used zero additional CPU ticks, 148476 KiB RSS,
eight threads, 41 FDs and the same process count, with four context switches.
After 100 maximize toggles it settled at 148420 KiB RSS/eight threads/41 FDs;
the synchronous dispatch burst took 64 ms, which is not presentation latency.

Earlier release samples varied: the default feature had four CPU ticks and the
configured-effects sample had fourteen ticks over three seconds with active
callback/context-switch traffic. A later sample after longer settling had no CPU
tick growth. The source of that transient activity was not isolated; these short
host-dependent samples are not long-run acceptance thresholds. Empty-session
startup also initialized driver threads and buffers, so its first two seconds
are startup measurements rather than a settled idle baseline. RSS excludes much
GPU/driver texture storage. Resource growth, hardware presentation, physical
input, direct-TTY/KMS, reboot and multi-monitor behavior remain unverified.
