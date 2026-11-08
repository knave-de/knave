# Shell performance and resource usage

Shell performance includes UI responsiveness, event-loop wakeups, IPC traffic,
Wayland subscriptions, scene updates, rendering work, and background lifecycle
tasks.

Before adding a timer, watcher, subscription, cache, buffer, task, or thread:

- define its owner, lifetime, cancellation, and cleanup behavior;
- bound event frequency, queue size, cache growth, and concurrency;
- avoid polling, duplicate subscriptions, and unbounded model or preview
  updates; and
- test disconnect, reload, shutdown, and compositor-unavailable paths.

Measure CPU, resident memory, threads, file descriptors, wakeups, and relevant
latency under idle, normal, and stress workloads. A shell that renders correctly
or passes unit tests can still wake continuously or retain GPU/IPC resources.

Record a baseline and expected delta for performance-sensitive changes. If no
baseline exists, establish one before declaring the change complete. Report live
Wayland, GPU, and installed-session measurements separately from build and unit
test results.

## Scheduling and current bounds

The runtime uses SCTK's calloop integration and a one-slot wake channel. The
Wayland loop blocks while idle. Desktop state arrives through a dedicated API
subscription; only disconnected clients use retry backoff. The retained overview
has one bounded pane IPC worker, replacing its former PNG capture worker. It also owns one loader thread, started on the first search keystroke, for the
application catalog and icons: a 32-job queue, no timers, woken results only, and
dropped without joining. Its
single pending request contains at most three workspace rectangles. User
actions have a separate one-slot worker so browsing updates cannot occupy the
action queue. The compositor renders window textures in those rectangles during
its ordinary output frame and paces inactive pane window frame callbacks at the
output refresh interval. Search and minimized controls remain in the shell.

The non-application overview host still uses the PNG preview query. The retained
overview no longer stores or decodes preview bitmaps. The renderer still bounds
text, display-list, and other image
resources. This change has not been measured in a live session: CPU, resident
memory, threads, file descriptors, wakeups, output latency, and GPU time are
unknown. The previous PNG path took roughly 35 ms at 1080p, 80 ms at 1440p,
and 149 ms at 4K per request in one local capture measurement; those figures
are a historical comparison, not a measured improvement for this implementation.

## Nested baseline

A release nested-session sample on 2026-09-26 used Winit, the bar role, no
overview service, and six one-second process samples after two seconds of
startup. The sampled process-lifetime CPU values settled from 2.4% to 0.8% for
Villain and from 8.0% to 2.4% for Knave Shell; Knave stayed at 0.0%. RSS stayed
around 121 MiB for Villain, 189 MiB for Knave Shell, and 3 MiB for Knave.
The process counts were 9, 38, and 1 thread respectively. The run shut down
without project child processes remaining. These are host-specific nested
baselines, not acceptance thresholds; direct TTY/DRM/GPU behavior and visual
latency remain unverified.

The renderer-foundation work adds actual system-font shaping and GPU readback
coverage, but has no updated idle/normal/stress resource measurements yet. The
offscreen test establishes rendering correctness on the available Intel
integrated GPU through Vulkan; it is not a performance baseline or a live
Wayland smoke test. One standalone offscreen test process used 0.163s user CPU,
0.194s system CPU, and 192.7 MiB peak RSS over 0.35s on 2026-09-27. That includes
WGPU initialization and driver/test-harness overhead; it is not comparable to
the nested-shell baseline above.
