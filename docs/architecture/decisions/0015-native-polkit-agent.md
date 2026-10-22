# ADR 0015: Native Polkit authentication agent

Knave needs authentication for desktop actions without depending on another
shell's agent. Knave Session now supervises `knave-shell polkit-agent` for the
whole session and checks `--supports-polkit-agent` before startup. Quick settings
no longer discovers or launches third-party agents.

The shell owns presentation and transport to the system Polkit authority.
GLib/GIO and libpolkit handle registration and PAM helper conversations on one
owned worker thread. One pending request and one queued command bound work;
additional requests fail explicitly. Only the authority's current unique bus
owner can invoke requests. Offered Unix user identities constrain selection.
Cancellation completes the pending invocation and stops its helper. Authority
replacement cancels old cookies and re-registers; setup/system-bus failures retry
after five seconds, with identical diagnostics suppressed. Healthy idle operation
has no polling timer. Dropping the UI closes the queue, stops the worker, and
unregisters; process exit also releases bus ownership and helper pipes.

A centered overlay takes exclusive keyboard input only while visible. Hidden
responses never enter retained scenes, clipboard exports, or IME surrounding
text. Response buffers are bounded to 4096 bytes and zeroized on replacement,
submission, focus loss and cancellation. This does not promise locked memory or
erasure of temporary copies made by platform libraries. PAM and polkitd alone
decide authorization; a successful D-Bus conversation reply does not grant it.

No configuration, desktop API or compositor protocol changes. Package versions
remain 0.1.0; deploy these Knave and Shell revisions together. Existing agents
are not displaced: registration errors are logged. Rollback both repositories
together to restore the previous on-demand third-party fallback.

Verification covers masked input, cancellation, queue/response limits, private
D-Bus sender checks and request lifecycle, plus supervisor child reaping. The
isolated Winit harness checks rendering and input separately. A nested desktop
without a logind session cannot establish real PAM authorization, direct TTY,
or hardware GPU coverage; these require review in a dedicated logind session.
