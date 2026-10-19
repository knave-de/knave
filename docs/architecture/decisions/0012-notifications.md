# ADR 0012: session notifications and bounded history

## Decision

Knave Session owns one notification service on one cancellable Tokio runtime
thread. It implements `org.freedesktop.Notifications`; Knave Shell owns the bar
bell, passive banners and the interactive history panel. Villain remains the
owner of input/focus policy. No compositor or portal contract changes are needed.

Ownership is first-acquirer-wins: request the standard bus name with DoNotQueue,
without ReplaceExisting or AllowReplacement. An already running daemon is left
alone. Knave never queues, retries acquisition, stops another daemon or changes
its activation files. Session shutdown releases only Knave's own connection.

The private `org.knave.Notifications` interface at `/org/knave/Notifications`
exposes Snapshot (JSON), image-free Summary (JSON) for the bar, Command (JSON)
and an invalidation-only Changed signal.
Typed payloads live in `knave-desktop-api::notifications`. Shell clients subscribe
before their initial snapshot, use a one-slot snapshot mailbox and a 16-command
queue, reconnect with capped backoff, and cancel their worker on shutdown.
Invalidation bursts are coalesced; no healthy connection polls. Standard methods, signals and external application
IDs follow the freedesktop notification protocol; the private contract has no
independent version. Ship matching Knave and Shell revisions.

## State and limits

History and DND runtime state are stored transactionally in
`$XDG_STATE_HOME/knave/notifications.sqlite3` (fallback `~/.local/state/knave/`).
Directory/file permissions are 0700/0600. An exclusive advisory lock prevents
independent buses from concurrently rewriting the same user's history. SQLite
secure deletion and an 8192-page limit bound storage. Failed/corrupt storage is
reported; it is never silently reset or replaced with an empty history.

Defaults retain 200 entries for seven days, oldest removed first. Configuration
allows 1–500 entries, 1–365 days, 1–3 banners, and a 1000–60000ms default banner
lifetime. At most 64 protocol notifications are live. Titles, body text, app IDs,
icons, action keys and labels are bounded. There are at most eight action pairs.
Malformed pairs, duplicate keys, invalid timeout values and foreign-sender
replacement/closure are rejected. Unknown hints are ignored. Progress is clamped
to 0–100. Replacements keep their ID and replace rather than duplicate the entry.

Banner visibility, unread state and protocol lifetime are independent. Ordinary
banner timeout preserves unread history and invalidates expired actions.
Transient hints exclude entries from durable history, the center and its badge.
Resident hints retain live entries after actions. An application exiting removes
its action buttons but leaves fire-and-forget banners visible. Restoration never
replays banners or revives actions, and protocol IDs are not reused after restart.
The service advertises actions, body and static icons; it does not claim sound,
inline replies, markup or protocol persistence capabilities.

Standard `image-data`, `image-path` and legacy underscore/icon-data hints supply
an image separately from the application icon. Raw RGB/RGBA validates dimensions,
8-bit samples, channels, rowstride and byte count. Paths support local absolute
files and local `file://` URIs in PNG/JPEG/WebP formats; remote URLs and themed
image-path names are ignored. Invalid images leave text delivery intact.
Two bounded blocking decoder jobs capture temporary files before Notify returns;
when both are busy, new notifications retain text without a preview. Files/raw
inputs are capped at 4 MiB and 4096 pixels per edge; file decoding allows at most
64 MiB per job. Shutdown waits at most one second for decoder work. Static
previews preserve aspect ratio, alpha and sRGB pixels, with a maximum 320px edge.
Newest previews share a 4 MiB decoded budget; older previews are discarded while
their text remains. Durable previews use a separate table in the same SQLite
transaction, leaving text rows readable by earlier builds. Previews are removed
with their notification and survive source-file deletion and session restart.
Snapshot image pixels use bounded base64; clients enforce a 32 MiB JSON limit.
The bar requests image-free Summary, and the shell reuses image textures across
text/read-state changes.

DND suppresses new banners while preserving history; disabling it does not replay
the backlog. Critical banners bypass DND only with `critical_in_dnd = true`.
DND is durable runtime state. Canonical per-app enabled/banners preferences live
under `[notifications.app."desktop-entry-id"]` in Knave's existing config.
Without a desktop-entry hint, the app name is used. These are client-supplied
presentation identities, not authenticated application permissions. Preferences,
limits and theme are loaded at session startup; the UI's per-app banner controls
write through `knave-config`, preserve unknown settings and apply immediately.

## Presentation

The 36px bar has a bell with a top-right unread badge (hidden at zero; 99+ cap),
a DND mark, and honest external/unavailable states. Clicking opens a right-side
center below the bar. It groups applications within each 20-entry page, supports
scrolling, long/wrapping content, actions, details, dismissal, mark-all-read,
clear-all and per-app banner muting. Visible entry headers become read; opening
the center does not mark unseen pages read. Page navigation remains available
outside the history scroll area. On short outputs the toolbar moves inside it.

One full-output transparent overlay draws banners and the center. Banners use
input regions intersected with their visible cards and request no keyboard.
The center temporarily requests exclusive keyboard input, closes on Escape,
focus loss or an outside click, and unmaps when idle to restore normal focus.
Shell's host exposes dynamic input regions, keyboard mode and background-update
draining; existing applications retain their defaults. Stable element IDs and
retained tree replacement preserve scroll, focus and pointer captures on updates.
Light, dark and high-contrast palettes derive from Knave appearance settings.
Image previews use square corners, fit within card width and are capped at 180px
in banners or 220px in history; long cards remain scrollable.
When an external daemon owns delivery, the bell has no numeric badge and the
center clearly identifies saved history as Knave-only. No external count or
history is fabricated.

## Verification and rollback

Model tests cover lifetime, DND, retention, restoration, sender exit, bounded
resources and transient/resident behavior. Private-bus tests cover the real
protocol, first-owner-wins, no queue, attempted takeover, sender identity,
actions, close reasons and malformed requests. Layout/input tests cover 500
entries, narrow/short/large surfaces, all palettes, lengthy content/action labels,
paging and updates during pointer capture. Run Cargo format, workspace tests,
strict Clippy and a coordinated managed build. Use the isolated desktop harness
to assert actual badge/banner/center behavior and record an MP4, plus idle,
normal and burst resource measurements. Nested rendering does not establish
TTY, hardware GPU or multi-monitor behavior. Sandbox portal notifications use the existing GTK Notification portal fallback;
a native Knave portal adapter is not introduced by this change.

Rollback uses the prior coordinated bundle. The additive config section and
history file can remain; disabling `notifications.enabled` starts no service or
notification shell role. The session projects the setting to shell children through
`KNAVE_NOTIFICATIONS_ENABLED`; disabled bars create no notification worker or button. Existing verified bundles and source checkouts are
not modified.
