# Overview service lifecycle

The persistent Overview crosses session, compositor and Shell process boundaries. Older Shell binaries cannot honor the initially hidden service contract, and disconnection or delayed unmapping must not trap input or cancel a newer opening.

Knave checks Shell support with `--supports-overview-service` before starting shell roles. The bounded probe exits without Wayland/GPU initialization and fails startup explicitly for unsupported binaries. Shell gates mapping on a live subscription, cancels unsent work on disconnect, processes current snapshots before action completions, and drains requests even while hidden. Villain clears visibility on unexpected mapped-to-unmapped transitions. A per-surface pending-hide bit consumes an unmap acknowledging an earlier hide without overwriting a newer reopen; initial bufferless negotiation never clears visibility.

The desktop contract and configuration schema stay unchanged. The capability
probe checks that the configured Shell supports the session-owned Overview
service. Install matching Knave, Villain, and Shell packages together;
configuration source remains untouched. No recurring activity is added;
startup/restart gains one probe bounded to two seconds, and compositor
bookkeeping uses one boolean per layer surface.

Regression coverage includes unsupported/hung probes, disconnect/reconnect, activation completion while hidden, unexpected unmap, and delayed hide/unmap after reopening. Winit checks establish nested runtime behavior; direct TTY/DRM and reboot require separate verification.
