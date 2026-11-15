# Shared GTK headers

## Decision and ownership

GTK header bars combine application UI with window controls. Villain can draw
its four controls in a reservation within that same row, leaving the app's
navigation, search, tabs, and menus intact. `shared_header = "gtk"` selects this
arrangement through Knave's existing window rules; `"off"` overrides an earlier
selection. There are no application IDs or environment switches in the runtime.
The default is off. Placement rules still run on open; shared-header presentation
is evaluated against current rules and takes effect on reload.

The GTK 3 and GTK 4/libadwaita adapters in `scripts/fixtures/shared-header/`
reserve 100 logical pixels on the right and normalize standard headers to 46px.
Villain draws a transparent 96 × 34 control strip 6px below the header's top.
It owns pointer motion and button actions inside that strip. The application
owns everything else, including header dragging. No second row or frame height
is added. This remains an explicit CSS integration, not toolkit detection or a
negotiated shared-header protocol. Custom app headers need separate validation.
Do not use a catch-all rule for arbitrary client-decorated applications.

## Configuration and rollout

```toml
[[compositor.window_rules.application]]
app_id = "org.gnome.Evince"
shared_header = "gtk"

[[compositor.window_rules.application]]
app_id = "com.github.johnfactotum.Foliate"
shared_header = "gtk"
```

The compositor supports the same standard GTK profile for other app IDs without
code changes. Install the matching `gtk.css` adapter and `settings.ini` in the
session's `XDG_CONFIG_HOME/gtk-3.0/` and `gtk-4.0/` respectively. Preserve existing
user CSS/settings when integrating these fragments. Hide app-drawn buttons with
`gtk-decoration-layout=:`. Knave Session locks the corresponding desktop
`button-layout` GSettings preference to `':'`; see [the session policy](0019-session-window-controls-policy.md).
These adapters affect standard headers in their GTK session; enable rules only
alongside a verified reservation. Production-wide automatic integration remains
future work. Transient, fullscreen, server-decorated, and undersized windows are
excluded. An overlapping native popup temporarily displaces shared controls.

Knave owns the additive typed rule field, parser, writer, and matching semantics;
Villain owns rendering and input. Shared config consumers rebuild together.
There is no schema version, wire/API change, data migration, or package bump.
To roll back, exit the Knave session to release its button-layout lock, then
remove the rules and GTK adapter fragments together and start the prior build.
Removing only rules/styles during an active policy session does not restore
application buttons. No development
artifacts are installed into the contributor's regular session.

## Theme and resource behavior

Decoration colors resolve Knave's selected light/dark mode, contrast, chroma,
and named theme CSS on config load/reload. System mode uses the portal fallback
in the same canonical config, with dark when no preference exists. Invalid CSS
rejects reload and preserves the active configuration. The renderer invalidates
its bounded control cache when the palette changes. It still uses Villain's
software painter; Knave Renderer integration remains separate.

GTK must receive the same desktop preference for the two owners to agree on
scheme. The settings portal can supply that preference; it does not guarantee
identical toolkit colors, geometry, or override an app's forced theme. This
change does not add a portal broker or an app-theme synchronization service.
No watcher, thread, timer, or queue is added. Named CSS edits require an explicit
Knave reload. Existing event-driven frames and bounded titlebar caching remain.

## Verification

After `cargo build --workspace --locked`, run:

```sh
python3 scripts/desktop-shared-header-smoke.py --run /tmp/shared-header-dark --display 97 --theme dark
python3 scripts/desktop-shared-header-smoke.py --run /tmp/shared-header-light --display 97 --theme light
```

Use a fresh run directory each time; add `--baseline` for an Evince resource
sample without compositor controls. The script expects installed Evince,
Foliate, and Arch's `speex`/`ijs` documentation PDFs. Tests set app-local libhandy/libadwaita color preferences and use GTK4’s Cairo
renderer to isolate geometry and contrast; this does not
establish real portal preference delivery. Sessions stop after automated checks.
Evidence includes binary identity, screenshots, assertions, resources, and MP4.
Nested software rendering does not establish hardware GPU or direct-TTY behavior.

Verified with the coordinated debug workspace artifact: 401 workspace tests,
workspace Clippy with warnings denied, and the explicitly enabled native
xdg-decoration negotiation test passed. Live Evince and Foliate checks covered
all four mouse actions in light/dark, two-window layout, search/menu operation,
fullscreen, and live rule disable/restore. Idle/normal/stress CPU, RSS, threads,
FDs, and context switches were recorded against an idle baseline. The baseline
and shared-header runs both held 14 compositor threads; short samples do not
establish long-run resource stability or exact scheduler wakeup counts.
