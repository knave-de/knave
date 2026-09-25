# Change-impact procedure

Before changing an architecture-affecting type, key, binary, protocol, or
build target, record:

- the owning component and change classification;
- every producer, consumer, and affected binary;
- public and private contract changes;
- configuration and migration impact;
- required version changes;
- tests proving compatibility; and
- rollout and rollback behavior.

The owning repository must inspect complete modules and all consumers before
editing. Cross-repository work uses linked branches and pull requests, keeps a
compatibility path where needed, and merges in dependency order.

For changes that add watchers, timers, subscriptions, background work, caches,
buffers, or parallelism, also record:
- the idle, normal, and stress workload behavior;
- CPU, memory, thread, process, file-descriptor, and wakeup impact;
- concurrency, queue, retry, and memory bounds; and
- cancellation, cleanup, debouncing, backoff, and failure behavior.

Do not accept a watcher or worker design merely because it is functionally
correct. A continuously waking watcher or unbounded parallel task can exhaust
CPU and memory while passing ordinary tests.

Each implementation slice is formatted, tested, linted, built, diff-checked,
and inspected before the next slice. Live Wayland, GPU, VT, process-lifecycle,
or installed-artifact behavior is reported as unverified until it has actually
been exercised.
