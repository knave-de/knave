# Shell component boundaries

This repository owns the desktop-facing shell implementation and its UI
composition. This workspace owns the shell binary, Wayland hosting, application
discovery and icon resolution. The standalone Knave UI repository owns the UI
toolkit and wgpu renderer; this workspace consumes both at one pinned Git revision.

The shell:

- consumes typed settings projections and desktop contracts from
  Knave;
- uses an explicit Wayland client and the Knave desktop API;
- owns presentation and interaction state; and
- reports lifecycle failures to the session owner.

It does not own compositor layout, focus, window-management, or input policy.
It does not write the canonical user configuration or reach through a public
contract into Villain internals. A shared contract change updates both sides in
the same coordinated package change.

The internal `knave-shell-ui` crate owns desktop snapshots projected into scenes,
desktop actions, workspace previews, status-bar composition and bundled desktop
assets. Both the Shell application and its Wayland host consume it. Generic UI
and renderer crates have no desktop-service dependency; `UiScene::display_list()`
is the Shell-owned adapter into generic drawing commands.

Standalone and Wayland hosts adopt the shared text-input purpose and context-menu
input contracts. Password surrounding text is withheld and the Wayland host sends
standard password/sensitive content hints. These are unreleased Rust API changes;
there is no Knave protocol, configuration or persisted-data migration. Keep the UI
pin and its Shell consumers together when deploying or rolling back.
