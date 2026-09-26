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

## Nested baseline

A release nested-session sample on 2026-09-26 used Knave with Villain in Winit
mode and the Rust/wgpu Shell bar. After two seconds of startup, six one-second
process samples recorded process-lifetime CPU values falling from 2.4% to 0.8%
for Villain and from 8.0% to 2.4% for Knave Shell; Knave stayed at 0.0%. RSS
remained around 121 MiB for Villain, 189 MiB for Knave Shell, and 3 MiB for
Knave. The processes used 9, 38, and 1 threads respectively. Shutdown left no
project child processes. These are host-specific nested baselines, not
acceptance thresholds; direct TTY/DRM/GPU behavior remains unverified.

Performance regressions require explanation, mitigation, or an explicit
reviewed exception. Do not hide them by weakening the workload or omitting the
measurement.
