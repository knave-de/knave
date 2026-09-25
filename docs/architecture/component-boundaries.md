# Component boundaries

Knave owns the user-facing desktop environment. Villain owns compositor and
window-manager behavior. The boundary is explicit even while both projects
are developed together.

| Component | Owns |
| --- | --- |
| `knave-session` | Session startup, supervision, shutdown, signals, and reaping |
| `knave-settings` | Typed access to the canonical Knave configuration |
| `knave-desktop-api` | Versioned public runtime desktop contracts |
| `knave-wayland` | Wayland client and layer-shell integration |
| `knave-renderer` | wgpu device, surfaces, rendering, and frame scheduling |
| `knave-ui` | Renderer-independent UI primitives and state transitions |
| `knave-shell` | Desktop-facing shell composition and interaction |
| Villain | Compositor, layout, focus, window management, and input policy |

Persistent configuration flows through Knave settings. Runtime state flows
through the session runtime directory or a typed service interface. Shell code
does not reach into Villain's private implementation, and Villain does not
write competing user-facing configuration.

The current Qt shell is transitional. The planned Rust/wgpu shell replaces its
implementation without changing ownership boundaries or silently changing the
public desktop contract.
