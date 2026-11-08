# Villain change-impact checklist

Before changing compositor, input, focus, layout, protocol, configuration
compatibility, or lifecycle code:

1. identify every affected surface, client, shell, session, and command;
2. inspect both sides of every protocol and every focus/input transition;
3. classify the change and record package, configuration, and rollback impact;
4. test normal, failure, shutdown, and stale-runtime-file paths; and
5. distinguish compiled/unit-tested behavior from live Wayland, GPU, VT, and
   installed-binary verification.

Cross-repository changes update every linked consumer together. Do not keep
obsolete API compatibility branches for unreleased package combinations.

If the change adds watchers, timers, subscriptions, background work, caches,
buffers, or parallelism, also record idle behavior, resource bounds,
cancellation and cleanup, and measured CPU, memory, thread, descriptor, and
wakeup impact under representative workloads. Check that work is not multiplied
unboundedly by clients, surfaces, outputs, workspaces, or reconnect attempts.

Do not accept a functionally correct event path that continuously wakes the
compositor or spawns unbounded work.
