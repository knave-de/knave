//! Passive banners and an interactive history panel on one input-shaped layer surface.
#[cfg(test)]
mod tests;
mod view;
use crate::overview::{
    icons::Icons,
    loader::{Done, Job, Loader},
};
use knave_desktop_api::notifications::*;
use knave_ui::{Color, DisplayList, Rect, toolkit::*};
use knave_wayland::{
    Application, KeyboardMode, SurfaceLayer, SurfaceOptions, notifications::NotificationClient,
};
use std::collections::{HashMap, HashSet};

const PANEL: ElementId = ElementId(1);
const CLOSE: u64 = 2;
const DND: u64 = 3;
const CLEAR: u64 = 4;
const READ: u64 = 5;
const PREVIOUS: u64 = 6;
const NEXT: u64 = 7;
const SCROLL: u64 = 8;
const PAGE: usize = 20;
#[derive(Clone)]
enum Target {
    Dismiss(u64),
    Invoke(u64, String),
    AppBanners(String, bool),
    Expand(u64),
}
pub struct Notifications {
    theme: Theme,
    pub(super) state: NotificationSnapshot,
    client: Option<NotificationClient>,
    loader: Option<Loader>,
    icons: Icons,
    previews: HashMap<u64, (NotificationImage, knave_ui::UiImage)>,
    scene: Scene,
    display: DisplayList,
    targets: HashMap<ElementId, Target>,
    size: [f32; 2],
    dirty: bool,
    connected: bool,
    error: Option<String>,
    page: usize,
    expanded: HashSet<u64>,
    seen: HashSet<u64>,
    regions: Vec<Rect>,
    pointer_down_outside: bool,
    pressed_card: Option<u64>,
    pressed: bool,
    pending_state: Option<NotificationSnapshot>,
    focus_on_open: bool,
    keyboard_focus_visible: bool,
}
impl Default for Notifications {
    fn default() -> Self {
        Self::new()
    }
}
impl Notifications {
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
            state: NotificationSnapshot::default(),
            client: None,
            loader: None,
            icons: Icons::default(),
            previews: HashMap::new(),
            scene,
            display: DisplayList::default(),
            targets: HashMap::new(),
            size: [0.0; 2],
            dirty: true,
            connected: false,
            error: None,
            page: 0,
            expanded: HashSet::new(),
            seen: HashSet::new(),
            regions: Vec::new(),
            pointer_down_outside: false,
            pressed_card: None,
            pressed: false,
            pending_state: None,
            focus_on_open: false,
            keyboard_focus_visible: false,
        }
    }
    fn send(&mut self, command: NotificationCommand) {
        if let Some(client) = &self.client
            && let Err(error) = client.command(command)
        {
            self.error = Some(error);
            self.dirty = true;
        }
    }
    fn apply(&mut self, state: NotificationSnapshot) {
        if !self.state.center_open && state.center_open {
            self.focus_on_open = true;
            self.keyboard_focus_visible = false;
            self.page = 0;
            self.seen.clear();
        }
        self.previews.retain(|id, (source, _)| {
            state
                .notifications
                .iter()
                .any(|n| n.id == *id && n.image.as_ref() == Some(source))
        });
        self.expanded
            .retain(|id| state.notifications.iter().any(|n| n.id == *id));
        self.seen
            .retain(|id| state.notifications.iter().any(|n| n.id == *id));
        self.page = self
            .page
            .min(state.history().count().saturating_sub(1) / PAGE);
        self.state = state;
        self.dirty = true;
        self.connected = true;
        self.error = None;
    }
    fn activate(&mut self, id: ElementId) {
        match id.0 {
            CLOSE => self.send(NotificationCommand::CloseCenter),
            DND => self.send(NotificationCommand::SetDnd {
                enabled: !self.state.dnd,
            }),
            CLEAR => self.send(NotificationCommand::Clear),
            READ => self.send(NotificationCommand::Seen {
                ids: self.state.notifications.iter().map(|n| n.id).collect(),
            }),
            PREVIOUS => {
                self.page = self.page.saturating_sub(1);
                self.dirty = true;
            }
            NEXT => {
                self.page =
                    (self.page + 1).min(self.state.history().count().saturating_sub(1) / PAGE);
                self.dirty = true;
            }
            _ => match self.targets.get(&id).cloned() {
                Some(Target::Dismiss(id)) => self.send(NotificationCommand::Dismiss { id }),
                Some(Target::Invoke(id, action)) => {
                    self.send(NotificationCommand::Invoke { id, action })
                }
                Some(Target::AppBanners(app_id, enabled)) => {
                    self.send(NotificationCommand::SetAppBanners { app_id, enabled });
                }
                Some(Target::Expand(id)) => {
                    if !self.expanded.remove(&id) {
                        self.expanded.insert(id);
                    }
                    self.dirty = true;
                }
                None => {}
            },
        }
    }
    fn mark_visible(&mut self) {
        if !self.state.center_open {
            return;
        }
        let Some(scroll) = self.scene.bounds(ElementId(SCROLL)) else {
            return;
        };
        let ids: Vec<_> = self
            .state
            .notifications
            .iter()
            .filter(|n| n.unread && !self.seen.contains(&n.id))
            .filter(|n| {
                self.scene
                    .bounds(ElementId(view::card_id(n.id)))
                    .is_some_and(|b| {
                        b.y >= scroll.y && b.y < scroll.y + scroll.height && b.height > 0.0
                    })
            })
            .map(|n| n.id)
            .collect();
        if !ids.is_empty()
            && let Some(client) = &self.client
        {
            match client.command(NotificationCommand::Seen { ids: ids.clone() }) {
                Ok(()) => self.seen.extend(ids),
                Err(error) => {
                    self.error = Some(error);
                    self.dirty = true;
                }
            }
        }
    }
}
impl Application for Notifications {
    fn theme_policy_changed(
        &mut self,
        policy: ThemePolicy,
        css: &str,
    ) -> Result<(), StylesheetError> {
        self.scene
            .set_theme_policy_stylesheets(policy, knave_shell_ui::styles::STYLESHEET, css)?;
        self.theme = policy.scheme;
        self.dirty = true;
        Ok(())
    }
    fn set_waker(&mut self, waker: knave_wayland::Waker) {
        self.loader = Some(Loader::start(
            knave_icons::IconLookup::from_process(),
            Some(waker.clone()),
        ));
        self.client = Some(NotificationClient::start(waker));
    }
    fn update(&mut self) {
        if let Some(result) = self.client.as_ref().and_then(NotificationClient::latest) {
            match result {
                Ok(state) => {
                    if self.pressed {
                        self.pending_state = Some(state);
                    } else {
                        self.apply(state);
                    }
                }
                Err(error) => {
                    self.connected = false;
                    self.state.center_open = false;
                    for n in &mut self.state.notifications {
                        n.banner = false;
                    }
                    self.error = Some(format!("Notifications unavailable: {error}"));
                    self.dirty = true;
                }
            }
        }
        if let Some(error) = self.client.as_ref().and_then(NotificationClient::error) {
            self.error = Some(error);
            self.dirty = true;
        }
        if let Some(loader) = &self.loader {
            for done in loader.take() {
                if let Done::Icon(name, image) = done {
                    self.icons.finish(name, image);
                    self.dirty = true;
                }
            }
        }
    }
    fn surface_visible(&self) -> bool {
        self.connected
            && (self.state.center_open || self.state.notifications.iter().any(|n| n.banner))
    }
    fn keyboard_mode(&self) -> Option<KeyboardMode> {
        Some(if self.state.center_open {
            KeyboardMode::Exclusive
        } else {
            KeyboardMode::None
        })
    }
    fn input_region(&self) -> Option<Vec<Rect>> {
        Some(if self.state.center_open {
            vec![Rect::new(0.0, 0.0, self.size[0], self.size[1])]
        } else {
            self.regions.clone()
        })
    }
    fn cursor(&self) -> CursorShape {
        self.scene.cursor()
    }
    fn input(&mut self, event: Input) {
        let keyboard_focus = match &event {
            Input::Key { pressed: true, .. } => Some(true),
            Input::PointerDown(_) => Some(false),
            _ => None,
        };
        if let Some(visible) = keyboard_focus
            && self.keyboard_focus_visible != visible
        {
            self.keyboard_focus_visible = visible;
            self.dirty = true;
        }
        if matches!(
            event,
            Input::Key {
                key: Key::Escape,
                pressed: true,
                ..
            }
        ) {
            self.send(NotificationCommand::CloseCenter);
            return;
        }
        if matches!(event, Input::FocusLost) && self.state.center_open {
            self.send(NotificationCommand::CloseCenter);
        }
        if let Input::PointerDown(p) = &event {
            self.pressed = true;
            self.pressed_card = self
                .state
                .notifications
                .iter()
                .filter(|n| n.live && n.actions.iter().any(|a| a.key == "default"))
                .find(|n| {
                    self.scene
                        .bounds(ElementId(view::card_id(n.id)))
                        .is_some_and(|r| contains(r, *p))
                })
                .map(|n| n.id);
            if self
                .targets
                .keys()
                .any(|id| self.scene.bounds(*id).is_some_and(|r| contains(r, *p)))
            {
                self.pressed_card = None;
            }
            self.pointer_down_outside = self.state.center_open
                && self.scene.bounds(PANEL).is_some_and(|r| !contains(r, *p));
        }
        let is_up = matches!(
            event,
            Input::PointerUp(_) | Input::Cancel | Input::FocusLost
        );
        let clicked_card = if let Input::PointerUp(p) = &event {
            self.pressed_card.filter(|id| {
                self.scene
                    .bounds(ElementId(view::card_id(*id)))
                    .is_some_and(|r| contains(r, *p))
            })
        } else {
            None
        };
        let result = self.scene.event(event);
        if result.action.is_none()
            && let Some(id) = clicked_card
        {
            self.send(NotificationCommand::Invoke {
                id,
                action: "default".into(),
            });
        }
        match result.action {
            Some(Action::Activated(id)) => self.activate(id),
            Some(Action::Toggled(id, checked)) if id == ElementId(DND) => {
                self.send(NotificationCommand::SetDnd { enabled: checked });
            }
            _ => {}
        }
        if is_up {
            if self.pointer_down_outside {
                self.send(NotificationCommand::CloseCenter);
            }
            self.pressed = false;
            self.pressed_card = None;
            self.pointer_down_outside = false;
            if let Some(state) = self.pending_state.take() {
                self.apply(state);
            }
        }
    }
    fn needs_frame(&self) -> bool {
        self.dirty
            || self.scene.needs_frame()
            || self
                .client
                .as_ref()
                .is_some_and(NotificationClient::has_update)
    }
    fn frame(&mut self, size: [f32; 2], text: &mut dyn TextMeasurer) -> &DisplayList {
        if self.dirty || self.size != size {
            let newly_open = self.focus_on_open;
            self.focus_on_open = false;
            self.size = size;
            let tree = self.compose();
            if let Err(error) = self.scene.replace(tree) {
                self.error = Some(error.to_string());
            }
            self.scene.layout(size, text);
            if newly_open {
                let _ = self.scene.set_focus(Some(ElementId(CLOSE)));
            }
            self.dirty = false;
        }
        self.display = self.scene.display_list(size, text).clone();
        self.display.clear_color = Color::rgba(0, 0, 0, 0);
        self.regions = if self.state.center_open {
            self.scene.bounds(PANEL).into_iter().collect()
        } else {
            self.state
                .notifications
                .iter()
                .filter(|n| n.banner)
                .filter_map(|n| self.scene.bounds(ElementId(view::card_id(n.id))))
                .filter_map(|r| {
                    r.intersection(Rect::new(0.0, 44.0, size[0], (size[1] - 44.0).max(0.0)))
                })
                .collect()
        };
        self.mark_visible();
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
            namespace: "knave-shell-notifications".into(),
            ..Default::default()
        },
        Notifications::with_theme(theme),
    )
    .map_err(|e| e.to_string())
}
fn age(created: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let minutes = now.saturating_sub(created) / 60000;
    if minutes == 0 {
        "Just now".into()
    } else if minutes < 60 {
        format!("{minutes}m ago")
    } else if minutes < 1440 {
        format!("{}h ago", minutes / 60)
    } else {
        format!("{}d ago", minutes / 1440)
    }
}
