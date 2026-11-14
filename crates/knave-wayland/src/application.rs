//! General shell surfaces sharing the production Wayland and rendering runtime.
use knave_ui::{
    DisplayList,
    toolkit::{Input, TextMeasurer},
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum KeyboardMode {
    None,
    #[default]
    OnDemand,
    Exclusive,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SurfaceLayer {
    Background,
    Bottom,
    #[default]
    Top,
    Overlay,
}
impl SurfaceLayer {
    pub(super) fn wayland(self) -> smithay_client_toolkit::shell::wlr_layer::Layer {
        use smithay_client_toolkit::shell::wlr_layer::Layer;
        match self {
            Self::Background => Layer::Background,
            Self::Bottom => Layer::Bottom,
            Self::Top => Layer::Top,
            Self::Overlay => Layer::Overlay,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SurfaceOptions {
    /// None fills the output. A size uses `position` in logical pixels.
    pub size: Option<[u32; 2]>,
    pub position: SurfacePosition,
    pub keyboard: KeyboardMode,
    pub layer: SurfaceLayer,
    pub namespace: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SurfacePosition {
    #[default]
    Center,
    TopRight,
}

impl SurfacePosition {
    pub(super) fn anchors(self) -> smithay_client_toolkit::shell::wlr_layer::Anchor {
        use smithay_client_toolkit::shell::wlr_layer::Anchor;
        match self {
            Self::Center => Anchor::empty(),
            Self::TopRight => Anchor::TOP | Anchor::RIGHT,
        }
    }
}
impl Default for SurfaceOptions {
    fn default() -> Self {
        Self {
            size: None,
            position: SurfacePosition::Center,
            keyboard: KeyboardMode::OnDemand,
            layer: SurfaceLayer::Top,
            namespace: "knave-ui".into(),
        }
    }
}

/// The application owns its scenes and values; the host owns GPU/Wayland resources.
/// Input is delivered in order. Painting is coalesced behind one frame callback.
#[derive(Clone, Debug)]
pub enum HostRequest {
    ToggleNotifications,
    ToggleQuickSettings,
    Applet(u64),
    Copy(String),
    Paste(knave_ui::toolkit::ElementId),
    Desktop(knave_desktop_api::DesktopCommand),
}

/// Wakes the host to draw after background work completes. Cheap to clone and safe to
/// call from any thread; a wake already queued absorbs further calls.
#[derive(Clone)]
pub struct Waker(super::WakeSender);
impl Waker {
    pub(super) fn new(sender: super::WakeSender) -> Self {
        Self(sender)
    }
    pub fn wake(&self) {
        let _ = self.0.try_send(super::RuntimeWake::Redraw);
    }
}

pub trait Application {
    /// Image-only surfaces can opt out of theme file notifications.
    fn uses_theme(&self) -> bool {
        true
    }

    /// Apply the selected user theme above shell presentation and below application CSS.
    fn theme_changed(
        &mut self,
        _theme: knave_ui::toolkit::Theme,
        _css: &str,
    ) -> Result<(), knave_ui::toolkit::StylesheetError> {
        Ok(())
    }

    /// Apply the selected scheme and accessibility palette. Legacy applications
    /// continue to receive the scheme through `theme_changed`.
    fn theme_policy_changed(
        &mut self,
        policy: knave_ui::toolkit::ThemePolicy,
        css: &str,
    ) -> Result<(), knave_ui::toolkit::StylesheetError> {
        self.theme_changed(policy.scheme, css)
    }

    /// Drain completed background work before visibility and rendering are evaluated.
    fn update(&mut self) {}
    /// Dynamic input regions in logical surface coordinates; None uses the whole surface.
    fn input_region(&self) -> Option<Vec<knave_ui::Rect>> {
        None
    }
    /// Override keyboard interaction when a passive surface opens an interactive panel.
    fn keyboard_mode(&self) -> Option<KeyboardMode> {
        None
    }

    /// Called once before the first frame. Applications that run background work
    /// keep the waker, and report finished results through `needs_frame`.
    fn set_waker(&mut self, _waker: Waker) {}
    /// Opt into one host-owned summary subscriber, stopped with the surface.
    fn uses_status_bar(&self) -> bool {
        false
    }
    fn notification_snapshot(
        &mut self,
        _snapshot: Option<&knave_desktop_api::notifications::NotificationSnapshot>,
    ) {
    }
    fn notifications_enabled(&mut self, _enabled: bool) {}
    fn applets(&mut self, _applets: &[knave_shell_ui::status_bar::Applet]) {}
    /// Opt into the host's bounded desktop snapshot, preview and action workers.
    fn uses_desktop(&self) -> bool {
        false
    }
    fn desktop_snapshot(&mut self, _snapshot: &knave_desktop_api::DesktopSnapshot) {}
    /// Resolved focused-window icon, delivered only while the matching focus is current.
    fn focused_app_icon(&mut self, _app_id: Option<&str>, _image: Option<&knave_ui::UiImage>) {}
    /// Request only the workspace previews currently visible to this application.
    /// Display-only compositor panes for this shell surface, in logical output pixels.
    fn overview_panes(&self) -> Option<Vec<knave_desktop_api::OverviewPane>> {
        None
    }
    fn preview_workspaces(&self) -> Option<Vec<knave_desktop_api::WorkspaceId>> {
        None
    }
    fn desktop_unavailable(&mut self) {
        self.host_error("Desktop connection unavailable; retrying");
    }
    fn workspace_previews(&mut self, _previews: &[knave_shell_ui::WorkspacePreviewImage]) {}
    fn desktop_action_finished(&mut self, _result: Result<(), String>) {}

    /// Queried after input and layout. Existing applications receive a visible arrow.
    fn cursor(&self) -> knave_ui::toolkit::CursorShape {
        knave_ui::toolkit::CursorShape::Default
    }

    fn text_input(&self) -> Option<knave_ui::toolkit::TextInputState> {
        None
    }
    fn take_request(&mut self) -> Option<HostRequest> {
        None
    }
    fn host_error(&mut self, _message: &str) {}

    /// Opt into full XKB symbols for key capture.
    fn wants_raw_keys(&self) -> bool {
        false
    }

    /// Receive the full XKB symbol for shortcut capture surfaces.
    fn raw_key(
        &mut self,
        _keysym: &str,
        _pressed: bool,
        _repeat: bool,
        _modifiers: knave_ui::toolkit::KeyModifiers,
    ) {
    }

    fn input(&mut self, event: Input);
    fn needs_frame(&self) -> bool;
    fn frame(&mut self, size: [f32; 2], text: &mut dyn TextMeasurer) -> &DisplayList;
    fn should_close(&self) -> bool {
        false
    }
    /// Whether the layer surface should have a committed buffer.
    fn surface_visible(&self) -> bool {
        true
    }
    /// Called after the host detaches a mapped surface's buffer.
    fn surface_unmapped(&mut self) {}
}

pub fn run_application(
    options: SurfaceOptions,
    app: impl Application + 'static,
) -> Result<(), super::WaylandError> {
    run_application_with_theme(options, app, knave_ui::toolkit::Theme::Dark)
}

pub fn run_application_with_theme(
    options: SurfaceOptions,
    app: impl Application + 'static,
    theme: knave_ui::toolkit::Theme,
) -> Result<(), super::WaylandError> {
    validate_options(&options)?;
    super::run_internal(
        super::ShellRole::Overview,
        Some((options, Box::new(app))),
        theme,
    )
}

fn validate_options(options: &SurfaceOptions) -> Result<(), super::WaylandError> {
    if options
        .size
        .is_some_and(|size| size.iter().any(|v| *v == 0 || *v > 16384))
    {
        return Err(super::WaylandError::InvalidOptions(
            "logical dimensions must be within 1..=16384",
        ));
    }
    if options.namespace.is_empty()
        || options.namespace.len() > 256
        || options.namespace.contains('\0')
    {
        return Err(super::WaylandError::InvalidOptions(
            "namespace must contain 1..=256 bytes without NUL",
        ));
    }
    Ok(())
}

pub(super) fn key(raw: u32) -> knave_ui::toolkit::Key {
    use knave_ui::toolkit::Key;
    match raw {
        0xff09 | 0xfe20 => Key::Tab,
        0xff0d | 0xff8d => Key::Enter,
        0x20 => Key::Space,
        0xff1b => Key::Escape,
        0xff51 => Key::Left,
        0xff52 => Key::Up,
        0xff53 => Key::Right,
        0xff54 => Key::Down,
        0xff50 => Key::Home,
        0xff57 => Key::End,
        0xff67 => Key::ContextMenu,
        0xffc7 => Key::F10,
        0xff08 => Key::Backspace,
        0xffff => Key::Delete,
        0x61 | 0x41 => Key::A,
        0x63 | 0x43 => Key::C,
        0x78 | 0x58 => Key::X,
        0x76 | 0x56 => Key::V,
        0x71 | 0x51 => Key::Q,
        0x6b | 0x4b => Key::K,
        _ => Key::Other,
    }
}

impl super::Runtime {
    pub(super) fn application_requests(&mut self, qh: &wayland_client::QueueHandle<Self>) {
        match self.app.as_mut().and_then(|app| app.take_request()) {
            Some(HostRequest::ToggleQuickSettings) => {
                self.dispatch_ui_action(knave_shell_ui::UiAction::ToggleQuickSettings)
            }
            Some(HostRequest::ToggleNotifications) => {
                self.dispatch_ui_action(knave_shell_ui::UiAction::ToggleNotifications)
            }
            Some(HostRequest::Applet(id)) => {
                self.dispatch_ui_action(knave_shell_ui::UiAction::Applet(id))
            }
            Some(HostRequest::Copy(text)) => self.copy_text(text, qh),
            Some(HostRequest::Paste(target)) => self.request_paste(target, qh),
            Some(HostRequest::Desktop(command)) => {
                let result = self
                    .action_worker
                    .as_ref()
                    .ok_or_else(|| "Desktop service is not enabled".to_owned())
                    .and_then(|worker| worker.dispatch(command));
                if let Err(error) = result
                    && let Some(app) = &mut self.app
                {
                    app.desktop_action_finished(Err(error));
                    self.exit |= app.should_close();
                    self.scene_dirty = true;
                }
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_options_are_validated_before_connecting() {
        assert!(validate_options(&SurfaceOptions::default()).is_ok());
        assert!(
            validate_options(&SurfaceOptions {
                size: Some([0, 200]),
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            validate_options(&SurfaceOptions {
                namespace: "bad\0name".into(),
                ..Default::default()
            })
            .is_err()
        );
    }
}
