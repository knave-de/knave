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

Night Light settings and the selected custom charge percentage are validated and
atomically written to `[quick_settings]` in the canonical Knave configuration
(or its explicit KNAVE_CONFIG override). Unknown source data survives writes.
The charge slider covers 50–100% in one-percent steps and sends the selected
value only after Apply. Opening the panel or moving the slider writes no hardware.
The last applied selection is restored in the UI; startup does not authenticate
or replay a hardware write.

UPower's installed API exposes a preset toggle, not an arbitrary percentage setter.
Knave discovers the matching native battery and reads its policy. Before applying
a custom percentage it disables an active UPower preset, then writes only the
existing battery end-threshold attribute through the validated Knave helper when
privilege is needed. A failed preset disable prevents the custom write. This keeps
a stale preset from overwriting the custom choice. Threshold readback must match
before persistence/success; it can be a driver cache and does not establish physical
firmware enforcement. There is no forced 80% selection.

The session-owned Polkit agent handles UPower, the charge helper and logind
(ADR 0015). Quick settings unmaps while authentication is pending and returns to
the same charging page with success or failure feedback. Individual authenticated
operations are bounded to ninety seconds, and the Shell command budget covers
both preset-disable and custom-write authentication. Ordinary UPower queries are
bounded to three seconds. No new worker, timer or subscription is introduced.

The coordinated Knave/Shell contract exposes `driver_limit`, `configured_limit`
and optional UPower `charge_limit` metadata; `SetChargeLimit` takes `percent`.
`[quick_settings].charge_limit` remains a validated optional custom percentage.
Deploy matching consumers together; roll back the coordinated bundle together.
Neither software rollback nor restoring a UI selection restores a hardware policy
that another tool has changed.

Villain consumes the additive `SetNightLight` desktop command, validates 2500–6500 K,
and applies an output-wide GLES RGB transform on both backends. The disabled path
has no additional render pass; the enabled path keeps one bounded output texture
and shader. Screenshots and portal source textures retain original colors.
This is independent of hyprsunset and requires no external gamma-control daemon.

Deploy the matching Knave, Shell and Villain commits together. Revert the three
components together to roll back; configuration and firmware thresholds are not
deleted. Nested verification covers software rendering and lifecycle, not physical
battery charging, radio devices, authentication agents or DRM/KMS hardware.
