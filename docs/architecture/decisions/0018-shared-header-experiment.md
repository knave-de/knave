# Shared GTK header experiment

## Context and decision

GTK headers combine application controls with client-side window decorations.
A second compositor titlebar wastes height; replacing the header loses app UI.
This opt-in experiment reserves the rightmost 96 × 34 logical pixels of Evince's
existing 46px header (6px below its top) with an isolated GTK CSS adapter.
Villain paints only its four window controls in that reservation and handles their clicks. Evince retains
navigation, menus, title, and header dragging. Window geometry is unchanged.

This is a fixed reservation for a cooperating test application, not automatic
GTK integration or a negotiated shared-header protocol. It reuses Villain's
software decoration painter; moving drawing to Knave Renderer remains separate.
Knave UI and Renderer gain no compositor dependency.

## Scope and lifecycle

Set `VILLAIN_EXPERIMENTAL_SHARED_HEADER_APP_ID=org.gnome.Evince` only in the
isolated compositor session. Copy `scripts/fixtures/shared-header-evince/gtk.css`
and `settings.ini` into that session's `XDG_CONFIG_HOME/gtk-3.0/`, and launch
Evince with `GTK_THEME=Adwaita:dark`. The native Wayland app ID must match exactly.
Set the same private session’s `button-layout` GSettings value to `:` before
launching Evince; GTK’s settings service can override `settings.ini`.
Transient windows, fullscreen, server-decorated windows, and windows smaller
than the reservation receive no shared controls. Controls temporarily disappear
while a native popup overlaps their reservation so they cannot obscure or intercept its menu.

The compositor owns actions, geometry, and input. GTK owns the reserved layout
and the remaining application content. There is no configuration schema,
package revision, IPC, or standard Wayland contract change. No new watcher,
thread, timer, or queue is created; painting uses the existing bounded titlebar
cache and frame scheduling. Removing the environment opt-in and isolated GTK
files restores existing behavior; ordinary sessions are unchanged.

## Verification

Build the coordinated workspace artifact and test in the nested desktop harness
with actual Evince documents. Verify application menu/search interaction,
float/tile, maximize/restore, minimize/restore, close, fullscreen, and multiple
windows. Capture screenshots and MP4 evidence, and compare compositor resources
with the opt-in absent. Software-rendered nested testing does not establish
hardware GPU or direct-TTY behavior.

Run `python3 scripts/desktop-shared-header-smoke.py --run /tmp/shared-header-test`
after a debug workspace build. Add `--baseline` with a fresh run directory to
measure the same Evince/CSS setup without compositor controls. Both runs stop
their desktops and applications; evidence remains in the chosen directories.
The smoke script expects Arch's `speex` and `ijs` documentation PDFs.

The initial coordinated workspace tests passed (400 tests, 23 ignored), followed
by the updated Villain tests (83 passed, 19 ignored), workspace Clippy with
warnings denied, and the actual Evince mouse-driven smoke test. Menu and search
screenshots were inspected. Initial three-second idle samples showed similar CPU
and context-switch activity with/without controls, with 14 compositor threads
in both cases. Short samples do not establish long-run resource behavior.
