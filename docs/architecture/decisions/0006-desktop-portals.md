# ADR 0006: User-local desktop portals

Knave owns session identity, canonical `[portal]` preferences and the private
`knave-portal-api` picker contract. Villain implements the standard
`ext-output-image-capture-source-v1` and `ext-image-copy-capture-v1` v1 protocols.
The portal backend owns D-Bus requests, source authorization, PipeWire streams,
preview generation and teardown. Shell owns source cards and active-sharing
controls using the existing Rust/wgpu toolkit. Desktop IPC remains API 1.3.

Install matching components together. Picker protocol v1 is private, bounded and
rejects incompatible versions. Source selection always requires fresh consent,
including restored sessions. No source is initially selected. One consent dialog
is allowed at a time; each has a 120-second deadline. A session owns its sharing
control; control failure/exit stops capture. Cancellation, session close, client
loss, compositor/PipeWire loss and backend shutdown release owned resources.

The supported descriptor exposes Settings v2, ScreenCast v6 (monitors, hidden or
embedded cursor), and Screenshot v2. Native PickColor returns NotSupported.
RemoteDesktop, InputCapture, window sources and cursor metadata remain deferred.
Settings can run without capture or input protocols. Generic environment aliases
and former `generic` restore vendors remain readable. Canonical appearance edits
use inotify and SettingChanged; invalid edits retain the last valid state.

The backend lives under `~/.local/libexec`, activation and routing defaults under
`~/.local/share`, and the user unit under `$XDG_CONFIG_HOME/systemd/user`. The
installer generates absolute paths and preserves user routing overrides. Direct
sessions export `Knave:Villain` and the resolved Shell executable; nested sessions
never import activation environment into the host bus. The backend is activated
on demand, without a frontend ordering cycle or enabled startup unit. Compositor
loss causes a successful backend exit, avoiding failure-restart loops.

Capture has a global eight-session/frame budget in Villain and eight streams in
the backend, at most 8 Mi pixels per frame (including 3840x2160), a four-command
PipeWire queue and four negotiated SHM buffers per stream. Frames use on-demand
33 ms timers; idle capture creates no timer. PipeWire waits on eventfd/events.
Output size changes require a backend restart and a new screencast session; live
stream renegotiation is not implemented. SHM GPU readback is the first supported path; DMA-BUF is deferred.

Verification uses separate runtime, config, data, cache, D-Bus and PipeWire services
and a nested Villain window. The smoke client checks raw capture, frontend
screenshots, ten nonempty buffers via its restricted PipeWire fd and session close.
Settings updates, cancellation and repeated-session resource snapshots are checked
by the backend's `scripts/smoke.py`. Native rendering is captured and inspected;
manual clicks, direct-TTY login, multiple physical monitors, OBS/browser and sandbox
apps require separate live verification. Successful compilation is insufficient.

Rollback restores the previous matching binaries and removes newly installed
Knave portal metadata (or runs `scripts/install.py --user --uninstall`). Existing
canonical config and routing overrides remain untouched. Start a fresh direct
session after rollout or rollback; host portals are not restarted for nested tests.
