# Knave Shell

Knave Shell is the Rust/wgpu user-facing shell for the independent Knave
Desktop Environment. It owns presentation and interaction surfaces while
Villain owns compositor policy, window state, focus, and input dispatch.
Winit is not a host-desktop integration: it is only a nested development
backend. The direct Wayland layer-shell path is the runtime target.

## Runtime

The optional development harness in `../knave/docs/desktop-testing.md` uses
`KNAVE_HARNESS_READY_DIR` for one-shot readiness markers. Configured shell services
signal after Wayland/renderer initialization and receipt of desktop state; the bar
also waits for a compositor frame callback. This private test handshake adds no
watcher and is inactive during normal sessions. Build this Shell revision before
running the matching harness; older Shell builds fail the harness readiness timeout.
Marker I/O failures propagate as typed runtime errors and a nonzero Shell exit.

    knave-shell bar
    knave-shell --supports-overview-service

The bar is a top layer with a 36-pixel exclusive zone. Its left edge carries the
icon-only bundled Knave mark; clicking it toggles Overview. Overview fills the
output, so both surfaces render the same complete `knave_shell_ui::status_bar::StatusBar`
component. Compact rounded groups place the logo, workspace chips, and focused
application together on the left, with tray applets, notifications, and icon-only
network/audio controls on the right. The network/audio group opens Quick Settings;
workspace occupancy dots, the active chip, unread count, and DND state follow the
existing desktop services. Overview routes the same hit targets.
Overview spacing and panel headers are compact; status, unavailable-service, and
administrator-authentication messages remain visible.
Overview also shows a
workspace carousel, search, and a minimized-window icon tray for the browsed
workspace. Left/Right, arrows, side previews, dots, and bar chips switch the
active workspace while keeping Overview open. Its preview and bar follow live
workspace changes. Click the center preview or press Enter to return to it. Click
a tray icon to restore its minimized window. Type or press Ctrl+K to search installed applications and press Enter to launch the top match, use Tab
to traverse controls, and Escape to hide overview. Super toggles
the session-owned Overview process. Entering the center workspace, restoring a
window, or launching an app hides Overview after the desktop service acknowledges
success. Each reopening starts with an empty search on the active workspace; the service keeps its app catalog and icon
cache. The overview displays
compositor-rendered workspace surfaces inside the cards;
the surfaces are visual only and card controls retain input. Both roles consume
Knave's desktop contract and keep desktop IPC off the Wayland frame thread.

Knave UI supplies the shell's colors, surfaces, typography, control states
and keyboard focus treatment. CSS in
`crates/knave-shell-ui/src/styles/` is limited to shell-specific layout details,
selected and error states, and the unread-count badge. User theme CSS layers over
these shell rules, from `$XDG_DATA_HOME/knave/themes/<theme-name>/dark.css` or
`light.css` (default data directory: `~/.local/share`). Configured theme and CSS
file changes apply live. Invalid edits retain the last valid theme; missing files
use the toolkit defaults. Application CSS overrides remain the final layer.
Layout and hit targets remain in the Rust compositions;
fullscreen structural roots and compositor preview regions use non-painting
Rust elements, so general theme rules cannot cover the desktop or previewed windows.

## Build

Building requires GLib/GIO and Polkit development libraries discoverable by
`pkg-config` (`glib-2.0`, `gio-2.0`, `polkit-gobject-1`, `polkit-agent-1`).


    cargo fmt --all -- --check
    cargo test --workspace
    cargo clippy --workspace --all-targets -- -D warnings
    cargo build --workspace --release --locked

## Desktop contract

The shell reads desktop state from Knave's JSON-lines API through
knave-desktop-api:

    $XDG_RUNTIME_DIR/knave/desktop-$WAYLAND_DISPLAY.sock

KNAVE_SOCKET overrides the derived path for isolated tests. The client uses one
dedicated subscription worker and a one-entry snapshot mailbox. Villain
pushes changed state; a healthy idle connection makes no periodic requests.
Only failed connections use reconnect backoff, capped at five seconds. It never
blocks the Wayland frame callback on desktop IPC. The Wayland loop blocks when
idle and redraws only for changed state, input, or preview completion; it does
not submit an unchanged frame continuously.

Input actions use a separate one-entry bounded queue and one worker. A full
queue drops an action with an explicit diagnostic instead of creating threads.

The retained overview uses one bounded geometry worker to send up to three
workspace pane rectangles. Transient failures retry with capped backoff, and
new geometry replaces an older pending request. Villain composes existing
window textures during normal output frames. The shell does not request,
decode, or upload PNGs for this overview. A legacy non-application overview host retains its preview worker.

The shell does not own persistent settings. It receives the compositor's
workspace/window state from Villain through Knave's public contract and sends
user actions back through that same contract.

## Install

    scripts/install.sh --user
    scripts/install.sh --system
    scripts/install.sh --prefix "$PWD/stage"

--user installs to ~/.local/bin and --system installs to /usr/local/bin,
requesting sudo only for the target prefix. --prefix is explicit and useful for
packaging. The installer also writes the README below the selected prefix.

## Implementation status

The supported runtime is Rust/wgpu over Wayland layer-shell. Overview uses the
retained UI toolkit; both hosts render the shared status bar display list. Individually interactive
window previews, fullscreen-hidden grouping, application launching/icons,
and multiple-output hosting remain future work. Fractional scale is consumed
through Wayland's fractional-scale and viewporter protocols when the compositor
advertises both; integer-scale compositors use the existing fallback.

## Workspace

- knave-shell: desktop surfaces and service integration;
- knave-wayland: direct layer-shell client and bounded desktop-state bridge;
- knave-apps and knave-icons: application discovery and icon resolution.

The UI toolkit and wgpu renderer live in the separate
[Knave UI repository](https://github.com/knave-de/knave-ui). Both library dependencies
are pinned to the same Git revision; a standalone Shell checkout fetches them through
Cargo. Use Knave's managed workflow with `--change knave-ui --change knaveshell`
for coordinated library/consumer changes.

### Native portal controls

`knave-shell shortcut-assignment` is launched by the global-shortcuts portal
when an application requests `ConfigureShortcuts`. It captures a modified XKB key
and returns the selected action and XDG trigger to the portal backend.

`knave-shell portal-picker` uses the existing retained UI toolkit and wgpu renderer
for source previews, explicit consent, cancellation and Stop sharing. The backend
launches it with the bounded JSON contract in `knave-portal-api`; it is
not a user-facing general CLI. No source is selected initially. For a single-screen
share, clicking a card labeled “Share this screen” selects and approves that screen
in one action. Workspace sets and screenshots retain selection plus confirmation. Screen, Window
and Workspaces tabs show choices permitted by the requesting application. Workspace
sets can be noncontiguous; the compact control says Frozen on excluded workspaces.
Active sharing uses a compact 350×36 control at the top-right edge of the bar,
with an app label and one-click Stop sharing. It never requests keyboard focus
and disappears when the portal session ends; it adds no watcher or timer.
Deploy matching Villain, Shell and portal binaries; see Knave ADR 0007.

Static wallpaper is configured through Knave's `[shell.wallpaper]` settings:
`image` is an absolute PNG path and `fit` is `cover`, `contain`, or `stretch`.
Knave Session starts the background role automatically; restart the session to
apply changes. Remove `image` for the black compositor background.

### Notifications

Knave Session starts `knave-shell notifications` alongside the bar. The bar bell
has a top-right unread badge, capped at 99+, and a DND marker. Click it for
scrollable, application-grouped history, actions, per-app banner muting, Do not
disturb and clear/read controls. Escape or an outside click closes the center.
Passive banners never request keyboard focus and leave the desktop outside their
cards clickable. History is paged in groups of 20 to bound retained UI size.
Long titles, bodies and action labels wrap; lengthy bodies have Show more/less.
The center and banners use Knave UI's bundled CSS for cards, controls and text;
Do not disturb is a switch above the history controls. Light/dark colors follow
Knave's appearance settings. The fullscreen dismissal host stays transparent
outside the bounded panel and banners.

The session service owns persistence and delivery. Expired banners remain in
history; restored history has no stale actions. Transient notifications do not
enter history or the count. If another daemon acquired notification delivery
first, the bell indicates external delivery and shows saved Knave history only.
Disconnected state is explicit. Match this Shell with the Knave revision supplying
[ADR 0012](../knave/docs/architecture/decisions/0012-notifications.md); the session
checks `--supports-notifications-service` before starting the role.

### Application applets

StatusNotifier tray icons appear immediately left of the notification bell in
both the bar and Overview. Left-click activates the app (or opens a menu-only
applet), right-click opens its menu, middle-click sends its secondary action,
and scrolling sends vertical scroll events. Menu loading runs independently of
tray refreshes, so an unresponsive applet cannot hold up opening another menu.
Exported dbusmenu menus support
submenus, disabled actions and check/radio states; Escape, Close or an outside
click dismisses the menu. Attention icons have a marker; passive items are hidden.

The shell hosts both KDE and freedesktop StatusNotifier interfaces on the
session bus and uses an existing KDE watcher when one is present. Registrations
are deduplicated and removed when their bus owner exits. One event-driven worker
per bar host owns a 16-command queue and at most 32 items. Icons that fit the output
are shown; narrow outputs retain space for the workspace label and bell. Legacy
XEmbed tray icons are not supported.

### Quick settings

Click the network/audio island beside the notification bell in the bar or
Overview for a native, full-height panel with grouped controls and connection tiles. It combines Ethernet/Wi-Fi, Bluetooth, output/microphone mute
and volume, brightness, power profiles, configurable battery charge limit, and
manual/scheduled Night Light. Audio devices and application volumes open in
pavucontrol; additional network profiles open in nm-connection-editor. Do not
disturb remains in Notifications. Missing hardware/services are shown explicitly.
The charge slider covers 50–100% in one-percent steps. Apply sends the selected
percentage and returns to the charging page with success or error feedback.
The saved selection and read-only driver report are shown separately. Driver
readback can be cached and does not prove physical charging enforcement.

The session starts the `quick-settings` role and its Knave-owned service. It needs
NetworkManager/nmcli, wpctl (WirePlumber), pavucontrol, brightnessctl,
powerprofilesctl and bluetoothctl for the corresponding controls. Battery charge
thresholds need kernel/firmware support and pkexec for root-owned attributes.
An active UPower preset is disabled before applying a custom percentage so it
does not replace the selected value. Knave starts its native `polkit-agent` shell role for authentication and releases the panel's
input while the prompt is active. Power lives in the main header and confirms in
a modal popup; subpages contain no Power footer or Refresh button. Brightness
supports 1–100%. There is no dependency on hyprsunset. Deploy matching
Knave, Shell and Villain revisions; see Knave ADR 0013.

### Authentication

Knave Session supervises `knave-shell polkit-agent` in direct desktop sessions. It registers
with the system Polkit authority for the current logind session, and shows an
compact password prompt only during authentication. The current account is
selected automatically when Polkit allows it. If policy requires another account,
the prompt names that account. Enter the password and press Enter or Continue. Escape or Cancel
cancels the request. Hidden responses are masked and excluded from clipboard
and IME surrounding text; Ctrl+A replaces the response and Backspace deletes.
No other desktop's agent is required. Registration failures are logged; stop
another agent before restarting Knave if one already owns the session. The
agent needs a system bus, Polkit with its authentication helper, and a logind
session. Winit sessions and Auto sessions with inherited display variables skip
the agent to protect the enclosing desktop. For a direct launch with inherited
display variables, select `backend = "tty"` explicitly.

Knave saves no password or authorization cache. Polkit owns temporary
authorizations and decides their lifetime from system policy. Inspect actual
cached authorizations and their expiration without opening a dialog:

    knave-shell polkit-cache

An empty cache prints `No cached authorizations.` Read failures are errors,
not an empty cache. The dialog shows retention information only when Polkit
explicitly supplies it; missing information is never presented as “not cached”.

Desktop scenes, actions, status-bar presentation and bundled desktop assets belong
to the internal `knave-shell-ui` crate. The renderer consumes generic display lists.
