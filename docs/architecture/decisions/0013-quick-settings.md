# 0013: Native quick settings

Knave Session owns `org.knave.QuickSettings` on the session bus. The typed JSON
snapshot and commands live in `knave-desktop-api::quick_settings`. Shell's
`quick-settings` role uses the retained knave-ui toolkit, shares the notification
panel geometry, and consumes this service through a bounded, cancellable worker.
Both the bar and Overview open the panel; Overview first hides itself. Power is a
menu in the main header only; its actions use a modal confirmation popup. Subpages
have no global footer or manual refresh controls. Brightness covers 1–100%.

NetworkManager owns connection credentials and routing. One Network entry prefers
an active Ethernet connection for presentation, with saved Wi-Fi connections and
Wi-Fi radio control beneath it. `nm-connection-editor` manages additional profiles.
Audio uses wpctl and pavucontrol, Bluetooth uses bluetoothctl, brightness uses
brightnessctl, and profiles use powerprofilesctl. Missing devices or tools are
explicit unavailable states. Power actions use logind and require confirmation.

The service serializes actions, caps subprocess output and duration, and reaps
children. While the panel is open it samples external state every three seconds;
when hidden it has no hardware polling. Scheduled Night Light wakes once per
minute. Shell never launches hardware commands on a frame/input thread.

Night Light settings are validated and atomically written to `[quick_settings]`
in the canonical Knave configuration (or its explicit KNAVE_CONFIG override).
Unknown source data survives writes. UPower owns battery charge-limit persistence
and its supported preset. Knave discovers the matching native power-supply battery,
reads its capabilities and calls `EnableChargeThreshold(bool)` on the system bus.
Reapply sends the same enabled value again, letting UPower restore its preset after
another tool changes the driver threshold. Unsupported and unavailable states are
explicit; there is no percentage slider, privileged Knave helper or sysfs write
fallback. The driver's read-only threshold is reported separately because some
drivers expose a cached request rather than the physical firmware cutoff.

The session-owned Polkit agent handles UPower and logind authentication (ADR 0015).
Quick settings unmaps while authentication is pending so its exclusive input cannot
block the authentication surface. Completion verifies UPower's reported enabled
state and reports errors; it does not establish physical charge enforcement.
Queries are bounded to three seconds and authenticated requests to ninety seconds.

The coordinated Knave/Shell contract replaces `Battery.limit` with `driver_limit`
and optional UPower `charge_limit` metadata; `SetChargeLimit` takes `enabled`.
The retired `[quick_settings].charge_limit` percentage is ignored and removed on
the next normal configuration write. No local charge setting is replayed at startup.
Deploy both consumers together; rolling back requires the previous coordinated
bundle and does not restore a retired percentage or change UPower's persisted state.

Villain consumes the additive `SetNightLight` desktop command, validates 2500–6500 K,
and applies an output-wide GLES RGB transform on both backends. The disabled path
has no additional render pass; the enabled path keeps one bounded output texture
and shader. Screenshots and portal source textures retain original colors.
This is independent of hyprsunset and requires no external gamma-control daemon.

Deploy the matching Knave, Shell and Villain commits together. Revert the three
components together to roll back; configuration and firmware thresholds are not
deleted. Nested verification covers software rendering and lifecycle, not physical
battery charging, radio devices, authentication agents or DRM/KMS hardware.
