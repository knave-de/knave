# Dynamic workspaces

## Context

Villain previously created ten fixed workspace slots. This left empty slots in
the user interface and imposed an arbitrary limit on the workspace list.

## Decision

`compositor.workspace.predefined` configures a contiguous, permanent prefix and
defaults to one. Villain keeps at least two slots, grows the list when a user
advances beyond its tail or selects a higher workspace, and retains every slot
through the highest occupied workspace plus one empty slot. It trims only empty
slots from the end, never below the predefined prefix plus its dynamic slot or
the active workspace. Forward navigation does not wrap; backward navigation
stops at the first workspace. The workspace list is bounded to 4096 slots to
keep memory use bounded.

Workspace IDs and the desktop snapshot remain one-based and list based. The
existing control protocol therefore needs no wire-format change. `MOD+1` through
`MOD+0` still address workspaces 1 through 10; custom bindings and forward
navigation reach higher workspaces.

## Impact and rollback

The change affects Knave configuration, Villain workspace state and navigation,
Shell workspace controls, and workspace capture discovery. Existing
configuration defaults to one predefined workspace. Set `predefined` to the
former fixed count to retain a permanent prefix of ten. Workspace screen capture
continues to use the protocol's 32-slot mask. Reverting the coordinated build
restores the fixed ten-workspace behavior.

## Verification

Update configuration, navigation, shell control and capture behavior together.
Run the coordinated Knave desktop build and isolated nested desktop harness;
direct TTY and hardware behavior require separate review.
