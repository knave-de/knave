# Shell change-impact checklist

Before changing shell UI, service code, renderer code, Wayland integration, or
an IPC message:

1. identify the owning crate/module and classify the change;
2. inspect all producers and consumers, including the compositor and session;
3. record public API, private protocol, configuration, and lifecycle effects;
4. update compatibility tests and migration notes when a contract changes; and
5. verify formatting, tests, lint, affected builds, diff cleanliness, and any
   required live smoke path.

If the change adds watchers, timers, subscriptions, model updates, caches,
buffers, or parallel work, also record idle behavior, resource bounds,
cancellation and cleanup, and measured CPU, memory, thread, descriptor, and
wakeup impact under representative workloads.

Keep settings migration, session supervision, protocol changes, and
old-tool removal in separate reviewable slices.
