# Desktop test harness

For coordinated worktrees and testing frozen binaries from all four repositories,
use the [development workflow](dev-workflow.md). `start --bundle /absolute/bundle`
selects those artifacts instead of mutable workspace targets and adds a private
portal activation/Settings check. `portal-check` repeats that assertion. This
private bus never imports environment into the host user manager. Ordinary
workspace mode below retains its existing portal-disabled behavior.

Use `scripts/desktop-harness.py` for feature verification and recorded demos.
It runs the workspace binaries in a private Xvfb/Winit desktop with software
rendering, isolated runtime/configuration and D-Bus. It never changes your normal
desktop configuration. Direct TTY/DRM, physical devices, hardware acceleration,
and portal activation need separate testing.
Mesa software rendering uses four worker threads; the video encoder uses two.

## Dependencies and builds

On Arch Linux:

```sh
sudo pacman -S --needed python xorg-server-xvfb xorg-xauth xdotool imagemagick ffmpeg x11vnc
```

Build the feature's affected components before starting; the harness does not
build or select installed copies from PATH:

```sh
cargo build --workspace --release --locked
(cd ../villain && cargo build --release --locked)
(cd ../knaveshell && cargo build --workspace --release --locked)
```

Use `start --profile debug` to select all three repositories' debug artifacts.
Execute desktop commands outside the Codex filesystem/network sandbox, with the
normal escalation mechanism. Python 3.11 or newer is required.

## Agent workflow

From the Knave repository, choose a new directory for each feature demonstration:

```sh
h=(python3 scripts/desktop-harness.py --run build/desktop-harness/my-feature)
"${h[@]}" start
"${h[@]}" record-start feature-demo
"${h[@]}" key Super_L
"${h[@]}" click 460 54
"${h[@]}" type kitty
"${h[@]}" key Return
"${h[@]}" snapshot
"${h[@]}" screenshot feature-result
"${h[@]}" record-stop
```

Coordinates refer to the 1280×800 screen; `click X Y --button 3` sends a right
click and buttons 4/5 scroll. `key` takes xdotool key names/combinations, such as
`Escape`, `ctrl+a`, and `Super_L+2`. Inspect screenshots and wait for the expected
desktop state between actions. After typing in a terminal, `key Return` executes
the command. A video records behavior; supplement it with assertions that prove
the intended result. State and input commands fail if a required process exits.

`start --config /path/to/config.toml` tests a private normalized copy of that
configuration. All source values are retained except the session backend,
binary paths, restart policy and portal enablement, which are forced for isolated
testing. Comments/formatting are not retained in the copy. Shell role settings
are respected. The original file is never modified.
Only an explicitly configured environment file is copied into the run directory;
an incidental sibling file named `environment` is not imported.

The matching Knave Shell build must support the private
`KNAVE_HARNESS_READY_DIR` handshake. The harness waits for per-role markers in its
fresh runtime directory, then retries a real control-socket status request until
the daemon is listening. Shell signals require configured Wayland/GPU state and
a desktop snapshot. The bar additionally waits for a compositor frame callback;
the hidden Overview signals when its configured service can handle desktop state.
An older Shell build times out and is cleaned up instead of reporting readiness.

Use `start --display 94 --vnc-port 5994` for another concurrent session. The
default display is :92, VNC port 5992. `--size 1600x900` changes the screen size.
`--no-vnc` supports agent-only tests without x11vnc. Existing run directories,
occupied displays/ports and existing artifact filenames are rejected.

## Test it yourself over SSH

Leave the harness running after the agent stops the recording. On the SSH host:

```sh
"${h[@]}" status
cat build/desktop-harness/my-feature/vnc-password.txt
```

On your own computer, open a tunnel (replace the SSH destination):

```sh
ssh -N -L 15992:127.0.0.1:5992 USER@SSH_HOST
```

Connect a VNC viewer to `127.0.0.1:15992` using the generated password. This is
the same desktop the agent controls; clicks and typing are live, and recordings
can include your testing. Coordinate control with the agent to avoid simultaneous
input. VNC listens only on IPv4 loopback and requires a random per-run password.
The X server uses a private Xauthority cookie and does not listen on TCP.
VNC password-file creation passes the secret through stdin, never process arguments.
The eight-character URL-safe password carries 48 bits of random entropy within
the legacy VNC authentication length limit.

When finished:

```sh
"${h[@]}" stop
```

Stop finalizes an active video and shuts down owned process groups, including
launched applications, then removes temporary sockets. Artifacts are retained.
Unexpected child exits also trigger cleanup. An abrupt host shutdown or SIGKILL
cannot guarantee MP4 finalization or cleanup; inspect the run logs in that case.

## Evidence and smoke test

Each run saves `manifest.json` (binary paths/hashes, repository commits and dirty
status), `config.toml`, component logs, `actions.jsonl`, screenshots and MP4s.
Recordings include the cursor, use H.264 at 20 fps, have no audio and are finalized
with metadata at the matching `.json` path. The run directory is private to your
user. Typed text is stored in the action log; avoid entering credentials in demos.

Run the actual desktop smoke test, including a recorded demonstration:

```sh
python3 scripts/desktop-harness-smoke.py --run build/desktop-harness/smoke-001
```

It opens Overview, searches/launches Kitty, verifies a command typed inside Kitty
by reading its output file, clicks between workspaces, validates the MP4 with
ffprobe, writes `smoke-result.json`, and stops its session. Kitty and its desktop
entry must be installed. It defaults to display :93 and VNC port 5993; both can be
overridden. For an interactive feature handoff, use the manual workflow and leave
the session running. Share the video path, the relevant assertions, and VNC tunnel
instructions with the user.

## Synthetic workspace gestures

Start with `--gesture-test` to opt into a private Winit-only gesture injector.
The harness removes inherited test flags; ordinary starts never enable it.
`gesture '{"op":"begin","fingers":3}'`,
`gesture '{"op":"update","x":-80,"y":0}'`, and
`gesture '{"op":"end","cancelled":false}'` call the same handlers as libinput.
`query` reports active workspace and current visual displacement; `remove`
simulates device removal. An optional `device` identifies an input stream.
This is test transport, not a desktop IPC/knavectl action.

After the standard `knave-dev test BUNDLE`, run the frozen feature runner:

```sh
python3 BUNDLE/tools/desktop-gesture-smoke.py --bundle BUNDLE --run /tmp/swipe-demo-001
```

It asserts pre-release displacement, both rendered workspace colors, stationary
panels, commit/snap-back/cancellation, reversal/wrapping, device isolation,
screenshot interruption, reload/disable/error handling, and bounded resource
counts. It saves screenshots, JSON assertions and an MP4 and stops its session.
Protocol tests additionally assert complete client streams and reserved stream
suppression. Synthetic delivery does not verify touchpad/libinput routing.

Pass `--skip-empty` to the frozen `desktop-gesture-smoke.py` runner to test the
optional `[compositor.workspace] skip_empty` policy. It populates workspaces
1, 4 and 7 and asserts shortcut/swipe skipping, wraparound and translated pixels
in both directions, minimized-only occupancy, direct empty selection, reload
validation, target destruction and the single/all-empty cases. It records
`workspace-skip-empty-demo.mp4` and restores two occupied slots for idle, normal
and cancellation-stress resource samples. Run the ordinary mode as well to
check that the default sequential behavior stays intact.

## Configurable charge-limit controls

After the standard bundle test, run the frozen charge-limit smoke runner:

```sh
python3 BUNDLE/tools/desktop-charge-limit-smoke.py --bundle BUNDLE --run /tmp/charge-limit-demo
```

It exercises the native slider and transport against a private Quick Settings
service, including several custom percentages, delayed replies, cancellation and
mismatched driver readback. The fixture never writes host battery settings.
Backend tests separately use a private UPower bus and temporary threshold files to
verify preset-disable ordering and custom writes. Screenshots, an MP4, assertions
and process resource samples are retained in the run directory. Firmware
enforcement and real Polkit authentication require physical-device review.
