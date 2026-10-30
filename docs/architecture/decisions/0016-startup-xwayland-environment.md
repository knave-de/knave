# ADR 0016: XWayland environment for startup commands

## Context

Knave Session owns startup-command children, while Villain owns the XWayland
server and learns its `DISPLAY` value asynchronously. Passing through the
session manager's inherited `DISPLAY` can target an enclosing desktop in nested
sessions; clearing it prevents X11 startup applications from connecting.

## Decision

Villain reports an explicit XWayland status through the existing desktop API.
When startup commands are configured, Knave waits for that status for up to
five seconds, then launches every entry with the session's `WAYLAND_DISPLAY`
and Villain's `DISPLAY` when available. It always removes inherited
`WAYLAND_SOCKET` and never preserves the caller's `DISPLAY`.

## Impact and rollback

The additive desktop query and response are implemented by Knave's API and
Villain. Knave Session consumes them; no shell or portal consumer changes.
The configuration format is unchanged. Old Villain builds do not understand
the new query, so deploy the matching Knave and Villain revisions together.
Reverting this decision restores Wayland-only startup commands and removes the
query from both contract and compositor.

## Verification

Run the managed coordinated build and nested desktop harness with both a
native Wayland startup command and an X11-only command such as `xterm`. A live
direct-TTY session remains necessary to establish hardware XWayland behavior.
