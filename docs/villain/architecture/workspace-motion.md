# Motion primitives and workspace presentation

Villain owns desktop workspace animation. This private behavioral change adds no
configuration, IPC or Wayland contract and needs no package-pin migration. Rollback
restores the preceding Villain binary. Shell overview browsing remains unchanged.

## Primitive

`motion::Tween` is a pure scalar timeline: start, goal, monotonic start instant and
duration. Explicit `sample(now)` returns position and completion, with cubic
ease-out interpolation. Zero duration, equal endpoints and late frames complete
exactly at the goal. Sampling never schedules work or mutates desktop state.
Retargeting samples at the supplied instant and starts from that position;
repeating the goal leaves the timeline alone. Position is continuous, velocity
need not be. Springs and configurable curves can be added behind a deliberate
motion policy later; they are not implicit guarantees of this primitive.

## Workspace ownership

Activation, history, layout and keyboard focus change immediately through the
existing switch path. Presentation tracks retain render-only workspace IDs and
horizontal offsets. Incoming windows are not moved/configured every animation
frame. Existing live workspace scenes provide windows and wallpaper; panels and
cursor remain stationary. Direct numbered/last-workspace selections use numeric
direction; next/previous preserve travel direction through skipped slots and
dynamic workspace creation.

New targets start adjacent to the departing active scene's sampled position.
Existing targets retarget from their sampled positions. Earlier outgoing tracks
finish independently, with the newest destination in front if motion overlaps.
There is at most one track per workspace (up to 4096), no event queue and no
captured framebuffer. Only scenes intersecting the output are composed; the
existing bounded scene-texture cache remains the resource owner. Repeated active
selection does not restart the presentation timeline.

Gesture tracking retains stream/device ownership and directly supplies offsets.
Release uses the same tween and scheduler as command-driven switches. A committed
gesture uses the shared logical activation path without launching another slide.
A new gesture cancels timed command presentation before taking ownership; an
explicit switch consumes an existing reserved gesture stream as before.

## Driver and interruptions

One cancellable calloop timer is shared by command slides and gesture settling,
paced to the selected output's advertised refresh (60 Hz fallback). It requests
repaints only while timed motion remains. Completion removes outgoing tracks and
the timer and restores pointer hit-testing. Client callbacks and commits include
all participating workspaces, while callbacks retain the existing refresh limit.
No worker, polling task or per-property timer is added. The 160 ms duration is
workspace policy, separate from interpolation and scheduling.

During timed motion ordinary window pointer hit-testing is suspended so a moving
image cannot receive input at its final layout coordinates. Panels remain usable;
a button or scroll completes timed motion before normal hit-testing/delivery.
Keyboard focus remains on the logically active workspace. Overview entry, resize,
reload, screenshot capture, host focus loss, seat pause, conflicting grabs,
render failure and shutdown discard motion and show the logical destination.
Device removal also cancels that device's gesture settle. Headless state tests
and pre-backend initialization do not start command presentation.

## Verification

Deterministic tests cover elapsed-time sampling, skipped frames, exact completion,
retargeting/reversal continuity, dynamic-tail navigation and track bounds. Real Wayland tests
cover logical focus/history, reserved gestures and timer cleanup. Frozen-bundle
nested checks must assert intermediate displacement, rendered source/destination,
stationary panels, rapid retargeting, interruptions and idle/stress resource bounds.
Nested Winit software rendering does not establish DRM/GPU or physical input behavior.

Run the feature check (ImageMagick and the standard harness dependencies required):

```sh
python3 scripts/workspace-motion-smoke.py --bundle /absolute/bundle --run /fresh/artifacts
```

Use `--baseline` with a preceding bundle to measure the same workloads without
requiring its test socket to expose the new motion fields.
