# Workspace swipes

Status: accepted for the coordinated workspace-swipe change.

Knave owns the additive typed `compositor.gesture` projection. Villain owns
input recognition, workspace transitions and rendering. Consumers are
knave-config's loader/validator/writer, Knave settings/session configuration
normalization, and Villain's runtime loader/reloader. Shell and portal rebuild
against the coordinated package set but receive no new desktop messages.
The current configuration format is unversioned. Omitted settings preserve
defaults; no stored data conversion is required. Deploy the configuration
contract before Villain's dependency pin.

Villain reserves configured swipe streams from begin, including vertical motion;
clients never receive a partial reserved stream. Unconfigured/disabled gestures
are forwarded. A frozen screenshot suppresses gestures; existing pointer grabs
prevent compositor recognition. An interruption consumes the remainder of a
reserved stream and discards its visual transition. Only one device owns a
stream; other devices cannot update or end it.

After 8 input units, horizontal movement must exceed vertical movement by 1.25x
to lock horizontal; vertical dominance rejects the reserved gesture. Each input
unit moves the workspace 4 logical pixels, clamped to one output width. Direction
can reverse. Left selects NextWorkspace, right PreviousWorkspace, with the same
wraparound as dispatch. Release at 25% of output width commits immediately via
the existing dispatch; otherwise it snaps back. Both settle for 160 ms with cubic
ease-out. Backend cancellation never commits. Wallpaper travels with windows;
other layer surfaces and cursor stay stationary. Overview blocks recognition.

Reload, explicit workspace switches, output resize, device removal, host focus
loss, seat pause, or a higher pointer/screenshot grab cancel active transitions.
Window destruction changes the existing layout scene without duplicating layout
policy. The animation uses one cancellable timer only while settling; finger
updates request repaint on demand. Incoming windows receive frame callbacks and
commits from either visible workspace trigger repaint.

Verification includes typed source round trips, recognizer transitions and
client protocol delivery, all-component checks via knave-dev, and a frozen-bundle
nested recording with intermediate displacement/state assertions. A private,
explicitly enabled Winit-only datagram injector feeds the production gesture
handlers; it does not prove physical libinput delivery or direct DRM/GPU behavior.
Measure idle, swipe and settle resource behavior. Rollback restores the previous
coordinated binaries; the additive source keys can remain for later upgrades.
