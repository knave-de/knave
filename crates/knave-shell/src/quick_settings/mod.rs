//! Native retained quick-settings panel. Only the transport worker touches system services.
#[cfg(test)]
mod tests;
mod view;
use knave_desktop_api::quick_settings::*;
use knave_ui::{Color, DisplayList, Rect, toolkit::*};
use knave_wayland::{
    Application, KeyboardMode, SurfaceLayer, SurfaceOptions, quick_settings::QuickSettingsClient,
};
use std::collections::HashMap;
const PANEL: ElementId = ElementId(1);
const CLOSE: u64 = 2;
const BACK: u64 = 3;
const OUTPUT: u64 = 4;
const INPUT: u64 = 5;
const BRIGHTNESS: u64 = 6;
const TEMPERATURE: u64 = 7;
const LIMIT: u64 = 8;
const START: u64 = 9;
const END: u64 = 10;
const POWER: u64 = 11;
const PROFILE: u64 = 15;
const CONFIRMATION: u64 = 12;
const CANCEL_CONFIRMATION: u64 = 13;
const CONFIRM_ACTION: u64 = 14;
#[derive(Clone, Copy, Default, PartialEq)]
enum Page {
    #[default]
    Main,
    Network,
    Night,
    Battery,
}
#[derive(Clone)]
enum Target {
    Page(Page),
    Command(QuickSettingsCommand),
    NightToggle,
    Schedule,
    ApplyLimit,
}
#[derive(Default)]
enum ChargeFeedback {
    #[default]
    Idle,
    Applying(u8),
    Applied(u8),
    Failed(String),
}
pub struct QuickSettings {
    theme: Theme,
    icon_color: Color,
    state: QuickSettingsSnapshot,
    client: Option<QuickSettingsClient>,
    scene: Scene,
    display: DisplayList,
    size: [f32; 2],
    dirty: bool,
    page: Page,
    targets: HashMap<ElementId, Target>,
    error: Option<String>,
    connected: bool,
    closing: bool,
    authenticating: bool,
    auth_unmap_focus_loss: bool,
    charge_feedback: ChargeFeedback,
    pressed: bool,
    pending: Option<QuickSettingsSnapshot>,
    outside: bool,
    focus_on_open: bool,
    limit: u8,
    limit_edited: bool,
    night: NightLight,
    confirmation: Option<QuickSettingsCommand>,
    next: u64,
}
impl Default for QuickSettings {
    fn default() -> Self {
        Self::new()
    }
}
impl QuickSettings {
    pub fn new() -> Self {
        Self::with_theme(Theme::Dark)
    }
    pub fn with_theme(theme: Theme) -> Self {
        let mut scene = Scene::new(Element::new(0, Widget::Panel)).unwrap();
        scene
            .set_theme_stylesheets(theme, knave_shell_ui::styles::STYLESHEET, "")
            .expect("panel accessibility stylesheet is valid");
        Self {
            theme,
            icon_color: view::text_color(theme.into(), "")
                .expect("default theme stylesheet is valid"),
            state: Default::default(),
            client: None,
            scene,
            display: Default::default(),
            size: [0.0; 2],
            dirty: true,
            page: Page::Main,
            targets: HashMap::new(),
            error: None,
            connected: false,
            closing: false,
            authenticating: false,
            auth_unmap_focus_loss: false,
            charge_feedback: ChargeFeedback::Idle,
            pressed: false,
            pending: None,
            outside: false,
            focus_on_open: false,
            limit: 100,
            limit_edited: false,
            night: Default::default(),
            confirmation: None,
            next: 100,
        }
    }
    fn send(&mut self, c: QuickSettingsCommand) {
        if matches!(c, QuickSettingsCommand::Close) {
            self.state.open = false;
            self.closing = true;
            self.dirty = true;
        }

        if let Some(client) = &self.client
            && let Err(e) = client.command(c)
        {
            self.error = Some(e);
            self.dirty = true
        }
    }
    fn apply(&mut self, mut state: QuickSettingsSnapshot) {
        if self.closing {
            if state.open {
                state.open = false;
            } else {
                self.closing = false;
            }
        }
        if !self.state.open && state.open {
            self.page = Page::Main;
            self.confirmation = None;
            self.charge_feedback = ChargeFeedback::Idle;
            self.focus_on_open = true;
            self.limit_edited = false;
        }
        if !self.limit_edited
            && let Some(battery) = &state.battery
        {
            self.limit = battery
                .configured_limit
                .or(battery.driver_limit)
                .unwrap_or(100)
                .clamp(50, 100);
        }
        if self.page != Page::Night {
            self.night = state.night
        }
        self.state = state;
        self.connected = true;
        self.dirty = true;
    }
    fn night_command(&self) -> QuickSettingsCommand {
        QuickSettingsCommand::SetNightLight {
            enabled: self.night.enabled,
            temperature: self.night.temperature,
            start: self.night.start,
            end: self.night.end,
        }
    }
    fn dismiss_confirmation(&mut self) {
        self.scene.close_popup();
        self.confirmation = None;
        self.dirty = true;
    }
    fn select_power(&mut self, action: u64) {
        self.confirmation = match action {
            1 => Some(QuickSettingsCommand::Suspend),
            2 => Some(QuickSettingsCommand::Reboot),
            3 => Some(QuickSettingsCommand::PowerOff),
            _ => None,
        };
        self.dirty = true;
    }
    fn activate(&mut self, id: ElementId) {
        if id.0 == CANCEL_CONFIRMATION {
            self.dismiss_confirmation();
            return;
        }
        if id.0 == CONFIRM_ACTION {
            if let Some(c) = self.confirmation.clone() {
                self.dismiss_confirmation();
                self.send(c);
                self.authenticating = self
                    .client
                    .as_ref()
                    .is_some_and(QuickSettingsClient::authentication_pending);
            }
            return;
        }
        if id.0 == CLOSE {
            self.send(QuickSettingsCommand::Close);
            return;
        }
        if id.0 == BACK {
            self.page = Page::Main;
            self.confirmation = None;
            self.dirty = true;
            return;
        }
        match self.targets.get(&id).cloned() {
            Some(Target::Page(page)) => {
                self.page = page;
                self.night = self.state.night;
                self.confirmation = None;
                self.dirty = true
            }
            Some(Target::Command(c)) => {
                self.error = None;
                self.send(c)
            }
            Some(Target::NightToggle) => {
                self.night.enabled = !self.night.enabled;
                self.send(self.night_command());
                self.dirty = true
            }
            Some(Target::Schedule) => {
                if self.night.start.is_some() {
                    self.night.start = None;
                    self.night.end = None
                } else {
                    self.night.start = Some(1140);
                    self.night.end = Some(420)
                }
                self.send(self.night_command());
                self.dirty = true
            }
            Some(Target::ApplyLimit) => self.apply_charge_limit(),
            None => {}
        }
    }
    fn apply_charge_limit(&mut self) {
        if matches!(self.charge_feedback, ChargeFeedback::Applying(_)) {
            return;
        }
        if !self
            .state
            .battery
            .as_ref()
            .is_some_and(|battery| battery.driver_limit.is_some())
        {
            return;
        }
        self.error = None;
        let result = self
            .client
            .as_ref()
            .ok_or_else(|| "Quick Settings service is unavailable".to_owned())
            .and_then(|c| {
                c.command(QuickSettingsCommand::SetChargeLimit {
                    percent: self.limit,
                })
            });
        self.charge_feedback = match result {
            Ok(()) => ChargeFeedback::Applying(self.limit),
            Err(e) => ChargeFeedback::Failed(e),
        };
        self.authenticating = self
            .client
            .as_ref()
            .is_some_and(QuickSettingsClient::authentication_pending);
        self.dirty = true;
    }
    fn finish_charge_limit(&mut self, result: Result<u8, String>) {
        let ChargeFeedback::Applying(requested) = self.charge_feedback else {
            return;
        };
        self.charge_feedback = match result {
            Ok(applied) if applied == requested => {
                if let Some(battery) = &mut self.state.battery {
                    battery.configured_limit = Some(requested);
                }
                self.limit = requested;
                self.limit_edited = false;
                ChargeFeedback::Applied(applied)
            }
            Ok(applied) => ChargeFeedback::Failed(format!(
                "Requested {requested}%, but driver reports {applied}%"
            )),
            Err(e) => ChargeFeedback::Failed(e),
        };
        self.authenticating = false;
        self.focus_on_open = self.state.open;
        self.dirty = true;
    }
    fn value(&mut self, id: ElementId, value: f64, commit: bool) {
        let v = value.round() as u8;
        let c = match id.0 {
            OUTPUT => {
                if let Some(s) = &mut self.state.output {
                    s.percent = v
                }
                Some(QuickSettingsCommand::SetVolume {
                    input: false,
                    percent: v,
                })
            }
            INPUT => {
                if let Some(s) = &mut self.state.input {
                    s.percent = v
                }
                Some(QuickSettingsCommand::SetVolume {
                    input: true,
                    percent: v,
                })
            }
            BRIGHTNESS => {
                self.state.brightness = Some(v);
                Some(QuickSettingsCommand::SetBrightness { percent: v })
            }
            TEMPERATURE => {
                self.night.temperature = value.round() as u16;
                Some(self.night_command())
            }
            LIMIT => {
                self.limit = v.clamp(50, 100);
                self.limit_edited = true;
                None
            }
            START => {
                self.night.start = Some(value.round() as u16);
                Some(self.night_command())
            }
            END => {
                self.night.end = Some(value.round() as u16);
                Some(self.night_command())
            }
            _ => None,
        };
        self.dirty = true;
        if commit && let Some(c) = c {
            self.send(c)
        }
    }
}
impl Application for QuickSettings {
    fn theme_policy_changed(
        &mut self,
        policy: ThemePolicy,
        css: &str,
    ) -> Result<(), StylesheetError> {
        self.scene
            .set_theme_policy_stylesheets(policy, knave_shell_ui::styles::STYLESHEET, css)?;
        let icon_color = view::text_color(policy, css)?;
        self.theme = policy.scheme;
        self.icon_color = icon_color;
        self.dirty = true;
        Ok(())
    }
    fn set_waker(&mut self, w: knave_wayland::Waker) {
        self.client = Some(QuickSettingsClient::start(w))
    }
    fn surface_unmapped(&mut self) {
        if self.authenticating {
            self.auth_unmap_focus_loss = true;
        }
    }
    fn update(&mut self) {
        if self.authenticating
            && self
                .client
                .as_ref()
                .is_none_or(|c| !c.authentication_pending())
        {
            self.authenticating = false;
            self.dirty = true;
        }
        if let Some(result) = self.client.as_ref().and_then(QuickSettingsClient::latest) {
            match result {
                Ok(s) => {
                    if self.pressed {
                        self.pending = Some(s)
                    } else {
                        self.apply(s)
                    }
                }
                Err(e) => {
                    if matches!(self.charge_feedback, ChargeFeedback::Applying(_)) {
                        self.finish_charge_limit(Err(e.clone()));
                    }
                    self.connected = false;
                    self.state.open = false;
                    self.error = Some(e);
                    self.dirty = true
                }
            }
        }
        if let Some(result) = self
            .client
            .as_ref()
            .and_then(QuickSettingsClient::take_charge_result)
        {
            self.finish_charge_limit(result);
        }
        if let Some(error) = self.client.as_ref().and_then(QuickSettingsClient::error) {
            self.error = Some(error);
            self.dirty = true;
        }
    }
    fn surface_visible(&self) -> bool {
        self.connected && self.state.open && !self.authenticating
    }
    fn keyboard_mode(&self) -> Option<KeyboardMode> {
        Some(if self.surface_visible() {
            KeyboardMode::Exclusive
        } else {
            KeyboardMode::None
        })
    }
    fn input_region(&self) -> Option<Vec<Rect>> {
        Some(if self.surface_visible() {
            vec![Rect::new(0.0, 0.0, self.size[0], self.size[1])]
        } else {
            Vec::new()
        })
    }
    fn cursor(&self) -> CursorShape {
        self.scene.cursor()
    }
    fn input(&mut self, event: Input) {
        if matches!(
            event,
            Input::Key {
                key: Key::Escape,
                pressed: true,
                ..
            }
        ) {
            if self.confirmation.is_some() {
                self.dismiss_confirmation();
                return;
            }
            if self
                .scene
                .interaction(ElementId(POWER))
                .is_some_and(|s| s.expanded)
            {
                self.scene.event(event);
                return;
            }
            if self.page == Page::Main {
                self.send(QuickSettingsCommand::Close)
            } else {
                self.page = Page::Main;
                self.confirmation = None;
                self.dirty = true
            }
            return;
        }
        if matches!(event, Input::FocusGained) {
            self.auth_unmap_focus_loss = false;
        }
        if matches!(event, Input::FocusLost) {
            if self.auth_unmap_focus_loss {
                self.auth_unmap_focus_loss = false;
            } else if self.state.open && !self.authenticating {
                self.send(QuickSettingsCommand::Close)
            }
        }
        if let Input::PointerDown(p) = &event {
            self.pressed = true;
            self.outside = self.confirmation.is_none()
                && self.scene.bounds(PANEL).is_some_and(|r| !contains(r, *p));
        }
        let release = matches!(
            event,
            Input::PointerUp(_) | Input::Cancel | Input::FocusLost
        );
        let result = self.scene.event(event);
        match result.action {
            Some(Action::Activated(id)) => self.activate(id),
            Some(Action::Selected(id, value)) if id.0 == POWER => self.select_power(value),
            Some(Action::Selected(id, value)) if id.0 == PROFILE => {
                if let Some(profile) = value
                    .checked_sub(1)
                    .and_then(|i| {
                        self.state
                            .power
                            .as_ref()
                            .and_then(|p| p.available.get(i as usize))
                    })
                    .cloned()
                {
                    self.send(QuickSettingsCommand::SetPowerProfile { profile });
                }
            }
            Some(Action::PopupClosed(_)) => self.dismiss_confirmation(),
            Some(Action::ValueChanged(id, v)) => self.value(id, v, !self.pressed),
            Some(Action::ValueCommitted(id, v)) => self.value(id, v, true),
            _ => {}
        }
        if release {
            if self.outside {
                self.send(QuickSettingsCommand::Close)
            }
            self.outside = false;
            self.pressed = false;
            if let Some(state) = self.pending.take() {
                self.apply(state)
            }
        }
    }
    fn needs_frame(&self) -> bool {
        self.dirty
            || self.scene.needs_frame()
            || self
                .client
                .as_ref()
                .is_some_and(QuickSettingsClient::has_update)
    }
    fn frame(&mut self, size: [f32; 2], text: &mut dyn TextMeasurer) -> &DisplayList {
        if self.dirty || self.size != size {
            self.size = size;
            let tree = self.compose();
            if let Err(e) = self.scene.replace(tree) {
                self.error = Some(e.to_string())
            }
            if self.confirmation.is_some() {
                let _ = self.scene.open_popup(ElementId(CONFIRMATION));
            }
            self.scene.layout(size, text);
            if self.focus_on_open {
                let _ = self.scene.set_focus(Some(ElementId(CLOSE)));
                self.focus_on_open = false
            }
            self.dirty = false
        }
        self.display = self.scene.display_list(size, text).clone();
        self.display.clear_color = Color::rgba(0, 0, 0, 0);
        &self.display
    }
}
fn contains(r: Rect, p: [f32; 2]) -> bool {
    p[0] >= r.x && p[0] < r.x + r.width && p[1] >= r.y && p[1] < r.y + r.height
}
pub fn run() -> Result<(), String> {
    run_with_theme(Theme::Dark)
}
pub fn run_with_theme(theme: Theme) -> Result<(), String> {
    knave_wayland::run_application(
        SurfaceOptions {
            keyboard: KeyboardMode::None,
            layer: SurfaceLayer::Overlay,
            namespace: "knave-shell-quick-settings".into(),
            ..Default::default()
        },
        QuickSettings::with_theme(theme),
    )
    .map_err(|e| e.to_string())
}
