# Performance and resource usage

Performance is a compatibility and reliability concern for the whole desktop,
not an optional cleanup step. A functionally correct watcher can consume a CPU
core if it wakes continuously. A small per-event allocation or unbounded worker
can multiply memory and thread usage until the session becomes unusable.

## Required design properties

Every watcher, timer, subscription, task, thread, process, cache, buffer, and
parallel operation must have:

- an explicit owner and lifetime;
- cancellation and cleanup on reload, failure, and shutdown;
- bounded concurrency, queues, retries, and memory growth; and
- defined idle, normal, degraded, and stress behavior.

Prefer event-driven notifications, debouncing, batching, and backoff. Avoid
busy loops, high-frequency polling, duplicate subscriptions, unbounded task
spawning, and retry storms.

## Review evidence

Performance-sensitive changes record a baseline and expected delta for CPU,
resident memory, threads, processes, file descriptors, wakeups, and relevant
latency. Measure idle, normal, and stress workloads. If a baseline or threshold
does not exist, record that uncertainty and establish one before declaring the
change complete.

The evidence may be a focused benchmark, profiler capture, resource sample, or
repeatable integration test. Compilation, unit-test success, and a short manual
smoke test do not establish acceptable resource behavior.

Performance regressions require explanation, mitigation, or an explicit
reviewed exception. Do not hide them by weakening the workload or omitting the
measurement.
