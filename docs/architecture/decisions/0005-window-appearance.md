# ADR 0005: Compositor window appearance

## Context and decision

Window geometry and visual effects must share one typed contract. Knave adds
optional schema-2 appearance settings; Villain implements gaps, border reservation,
rounded content/input masks, independent side shadows, focus-specific content
opacity/background blur, and independent per-view gates. Fullscreen always
bypasses effects. Defaults preserve the current compositor appearance. Settings
remain in Knave's canonical file; no session/Shell/API change or extra watcher,
worker, timer or subscription is introduced. See [the contract](../window-appearance.md).

## Consumers and compatibility

Consumers are knave-config, Knave CLI/session configuration loading, and Villain's
config reload, layout, split input, rendering, previews and overview panes.
Schema stays 2 because all fields are additive with defaults; existing schema-1
projection remains unchanged. Desktop API, shell protocol and binary/library
versions stay unchanged. Rust consumers constructing complete CompositorConfig
literals must initialize `appearance`. Older Knave preserves this unknown nested
TOML during ordinary writes; older Villain ignores the settings. No migration is
needed, and new Knave writes preserve unknown leaves inside appearance tables,
including inline tables. Invalid values reject loading/writing/reload atomically.

## Resource ownership

Disabled effects use Smithay's normal surface elements and incremental damage.
Enabled effects use GLES offscreen composition and a scene cache keyed by source
commit, geometry, focus and configuration. Cached output/preview scenes share one
LRU with at most ten entries and 36 Mi pixels (144 MiB RGBA texture storage).
Individual temporary targets are bounded to 36 Mi pixels; peak transient GPU
storage also includes live composition elements, window masks and blur intermediates
and a bounded amount of live window content. Lower scene elements are flattened
into one output texture before retained effect targets exceed the pixel budget. There is no background allocation loop.
Cache entries hold textures rather than clients; destruction and config reload
clear cached scenes, and replacement/eviction drops GPU references through Smithay.
Renderer/session teardown releases the cache and shaders. Shader work and blur
passes are bounded; scene changes can require full offscreen redraw, so measured
normal/stress cost is part of validation. Cached unchanged scenes avoid that work
on cursor-only repaints. Physical output behavior needs separate verification.

## Rollout and rollback

Publish the Knave config commit before deploying the matching Villain dependency
pin. Local validation before publication uses a Cargo patch pointing at this
Knave checkout; that machine-specific file is not committed. Deploy the two
feature commits together and restart Villain once. Shell binaries need no update.
Roll back Villain first, then Knave if desired. Appearance tables may remain:
older versions ignore them, preserve them during writes and use the old visuals.
No application bindings, source config or saved floating geometry are rewritten.

## Verification

Configuration tests cover omitted defaults, bounds, color parsing, round trips,
unknown nested/inline fields and rejected writes. Villain tests cover asymmetric
geometry, tiny outputs, divider gaps, maximization switches and fullscreen
precedence. Real isolated Wayland, layer-shell and XWayland tests exercise client
requests. Surfaceless GLES pixel tests cover border sides, independent corners,
content opacity, side shadows, blur extent and actual background color mixing.
Nested smoke checks, resource samples and unverified paths are recorded with the
feature result. Compilation alone does not validate direct-TTY presentation.
