# Window dragging and stack divider resizing

Villain owns input capture, hit testing, placement, focus, cursors and drag
rendering. Knave owns WindowTarget/ResizeEdge, the move-window, resize-window and
toggle-floating-focused commands, knavectl and the canonical binding configuration.
All desktop API consumers, including Shell and the portal backend, must build
against the matching Knave revision. No overview behavior changes in this slice.

One event-driven interaction per seat handles authenticated Wayland/X11 requests,
mouse bindings and modal keyboard/IPC starts. Protocol requests still validate
seat, serial/button and initiating surface. Compositor bindings do not fabricate
client serial authorization. A held-button drag ends on its initiating release;
modal starts commit on left release or Enter. Escape and lifecycle invalidation
cancel. Synthetic releases cancel first, preventing accidental tile drops.
Client buttons/keys are suppressed through their matching releases. Reload,
workspace/layout/output changes, focus loss and target destruction clean up
pointer grabs, cursor overrides and xdg resizing state.

MOD+left drags live tiled content above stable slots; a source placeholder and
destination tint provide feedback. Movement must exceed five logical pixels.
Drop swaps source/destination while retaining slot proportions; invalid drops
leave order unchanged. Floating moves keep the pointer anchor. Floating border
and five-pixel exterior hit regions support unmodified edge/corner resize;
content clicks, popups and upper shell layers keep their input. MOD+right is an
alternative. Resizes preserve the opposite edge and constrain client geometry
to usable output bounds, decorations and size hints.

The master divider retains its existing drag affordance. Horizontal stack
dividers resize only the adjacent pair, with a 64 logical-pixel slot minimum
(or equal shares on tiny outputs). Stable-ID window weights preserve proportions
across workspace switches and expanded states. Membership changes normalize
remaining weights; new stack members get the default share. MOD+V toggles
floating, preserves tile order and remembers the floating rectangle; an explicit
runtime override prevents client commits from undoing the user's selection.
All placement and proportions reset on restart; configuration is never rewritten.

Binding grammar additionally accepts BUTTON_LEFT, BUTTON_RIGHT and BUTTON_MIDDLE
with existing modifiers. Explicit combinations override their matching defaults.
Existing keyboard strings retain their meaning. Commands use the unversioned
shared desktop contract; no configuration schema or package version field is
added. Rollback deploys the matching previous package set and removes custom
bindings that name these new actions or mouse triggers.

Verification requires parser/contract tests, real Wayland and XWayland grabs,
rendered drag pixels, input isolation, commit/cancel paths, weighted coverage,
adjacent-only resizing, floating mode retention and bounds. The managed frozen
bundle and nested desktop harness establish coordinated behavior; physical
input, DRM/GPU, multi-output transfers and overview drops require later checks.
No timer, worker, subscription or persistent cache is added. Per-motion work and
idle/stress resource measurements are recorded with the feature evidence.
