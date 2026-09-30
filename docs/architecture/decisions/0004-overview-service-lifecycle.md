# Overview service lifecycle

The persistent Overview crosses session, compositor and Shell process boundaries. Older Shell binaries cannot honor the initially hidden service contract, and disconnection or delayed unmapping must not trap input or cancel a newer opening.

Knave checks Shell service contract 1 using `--supports-overview-service` before starting shell roles. The bounded probe exits without Wayland/GPU initialization and fails startup explicitly for unsupported binaries. Shell gates mapping on a live subscription, cancels unsent work on disconnect, processes current snapshots before action completions, and drains requests even while hidden. Villain clears visibility on unexpected mapped-to-unmapped transitions. A per-surface pending-hide bit consumes an unmap acknowledging an earlier hide without overwriting a newer reopen; initial bufferless negotiation never clears visibility.

Desktop API 1.4, subscription 1.3, and configuration schema 2 are unchanged. The capability flag is additive; older Shell binaries fail the check safely. Install matching Shell and Villain before starting the new Knave session manager. Roll back the matched binaries together; configuration source remains untouched. No recurring activity is added; startup/restart gains one probe bounded to two seconds, and compositor bookkeeping uses one boolean per layer surface.

Regression coverage includes unsupported/hung probes, disconnect/reconnect, activation completion while hidden, unexpected unmap, and delayed hide/unmap after reopening. Winit checks establish nested runtime behavior; direct TTY/DRM and reboot require separate verification.
