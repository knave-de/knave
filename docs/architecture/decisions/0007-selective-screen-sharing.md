# ADR 0007: Windows and allowed-workspace sharing

The portal picker must accept pointer input immediately and offer screen, window
and workspace-set modes with explicit consent. Multi-monitor behavior is out of scope.

Villain owns the capture boundary. Window enumeration and image sources use
standard `ext-foreign-toplevel-list-v1` and
`ext-foreign-toplevel-image-capture-source-v1`, including managed XWayland windows.
Window frames contain only the selected window, independent of stacking and the
active workspace. Closing that window stops its capture; another is never substituted.

Private `knave-workspace-capture-v1` sources have an immutable allowed-workspace
mask and use standard image-copy-capture sessions. They render only windows in
the active allowed workspace, excluding shell layers and overview panes. On an
excluded workspace, repeat the last allowed frame, including its frozen cursor
state; before any allowed frame exists, produce opaque black. Do not render
excluded content and filter it later in the backend. Retain at most one bounded
frame per session and release it on session destruction.

Knave owns picker schema and workspace protocol bindings. Picker v2 adds source
kinds and treats a workspace set as one capture target even for single-stream
applications. It accepts monitor-only v1 requests; old shells reject v2 explicitly.
Deploy matching backend and Shell for v2. Villain owns protocol implementation
and workspace policy. The portal owns source discovery, consent validation,
immutable stream targets, PipeWire and cancellation. Shell owns tabs, cards,
workspace checkboxes and the compact Stop control. No canonical settings migration
or public desktop IPC change is needed.

Advertise selective choices only when their managers are available. Never fall
back from a disappeared selective source to whole-screen capture. Budgets remain
bounded across sessions, frames, previews and source lists.

Verify first-click pointer input with the cursor already over the picker, window
capture behind another window and on another workspace, allowed/excluded switches
with distinguishable pixels, Stop/source destruction, OBS negotiation, repeated
cleanup, CPU/RSS samples and an MP4. Nested tests do not establish hardware GPU
or direct-TTY behavior.

Install the new compositor before matching portal and Shell. Rollback stops
selective sessions and restores matching prior binaries. Monitor-only clients and
session configuration remain usable.
