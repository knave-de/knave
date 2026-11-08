//! Workspace-first overview with a compact minimized-window icon tray.
pub(crate) mod icons;
pub(crate) mod loader;
mod view;
use knave_apps::{Catalog, Environment};
use knave_desktop_api::{
    DesktopCommand, DesktopSnapshot, OverviewPane, WindowId, WindowSummary, WorkspaceId,
};
use knave_icons::IconLookup;
use knave_ui::{DisplayList, toolkit::*};
use knave_wayland::{Application, HostRequest};
use loader::{Done, Job, Loader};
use std::collections::HashMap;

const SEARCH: ElementId = ElementId(2);
use knave_shell_ui::status_bar::{STATUS_BAR_HEIGHT, StatusBar};
const MAX_WINDOWS: usize = 1024;
const MAX_WORKSPACES: usize = 128;
const PAGE_SIZE: usize = 5;
const MAX_RESULTS: usize = 64;

#[derive(Clone, Copy)]
enum Target {
    Workspace(WorkspaceId),
    EnterWorkspace(WorkspaceId),
    StatusBar(knave_shell_ui::UiAction),
    Window(WindowId),
    App(usize),
    AllApps,
    Page(bool),
    DockWindow(WindowId),
    DockPage(bool),
}

/// The catalog loads on a worker after the first query or when a snapshot
/// first shows minimized windows, so neither startup nor typing ever waits on
/// the filesystem.
enum Apps {
    Unloaded,
    Loading,
    Ready(Catalog),
    Unavailable,
}

fn app_element(index: usize) -> ElementId {
    ElementId(0x300_0000_0000 + index as u64)
}

pub struct Overview {
    snapshot: Option<DesktopSnapshot>,
    notifications: Option<knave_desktop_api::notifications::NotificationSnapshot>,
    applets: Vec<knave_shell_ui::status_bar::Applet>,
    focused_icon: Option<knave_ui::UiImage>,
    theme: knave_ui::toolkit::Theme,
    notifications_enabled: bool,
    notifications_after_hide: bool,
    quick_settings_after_hide: bool,
    panes: Vec<OverviewPane>,
    workspace: Option<WorkspaceId>,
    scene: Scene,
    display: DisplayList,
    targets: HashMap<ElementId, Target>,
    window_ids: HashMap<WindowId, ElementId>,
    next_id: u64,
    query: TextEdit,
    apps: Apps,
    icons: icons::Icons,
    loader: Option<Loader>,
    /// Consumed when the worker starts.
    lookup: Option<IconLookup>,
    catalog_env: Option<Environment>,
    pinned_apps: Vec<String>,
    waker: Option<knave_wayland::Waker>,
    /// Enter arrived while the catalog was still loading.
    launch_when_ready: bool,
    page: usize,
    dock_page: usize,
    result_page: usize,
    all_apps: bool,
    connected: bool,
    size: [f32; 2],
    dirty: bool,
    pending: bool,
    hide_after_action: bool,
    request: Option<HostRequest>,
    error: Option<String>,
    surface_focused: bool,
    background_pressed: bool,
    pointer: Option<[f32; 2]>,
}
impl Default for Overview {
    fn default() -> Self {
        Self::new()
    }
}
impl Overview {
    fn status_bar_component(&self, width: f32) -> StatusBar {
        let mut bar = StatusBar::with_focused_icon(
            0,
            width,
            STATUS_BAR_HEIGHT,
            self.snapshot.as_ref(),
            self.notifications_enabled,
            self.notifications.as_ref(),
            &self.applets,
            self.focused_icon.as_ref(),
        );
        bar.set_user_policy(
            self.scene.theme_policy(),
            self.scene.user_theme_stylesheet(),
        )
        .expect("selected shell theme stylesheet is valid");
        bar
    }
    pub fn new() -> Self {
        Self::with_theme(knave_ui::toolkit::Theme::Dark)
    }
    pub fn with_theme(theme: knave_ui::toolkit::Theme) -> Self {
        let mut scene = Scene::new(Element::new(0, Widget::Panel)).expect("valid empty scene");
        scene
            .set_desktop_theme(theme, knave_shell_ui::styles::STYLESHEET)
            .expect("bundled shell theme stylesheet is valid");
        Self {
            display: DisplayList::default(),
            snapshot: None,
            notifications: None,
            applets: Vec::new(),
            focused_icon: None,
            theme,
            notifications_enabled: true,
            notifications_after_hide: false,
            quick_settings_after_hide: false,
            panes: Vec::new(),
            workspace: None,
            scene,
            targets: HashMap::new(),
            window_ids: HashMap::new(),
            next_id: 100_000,
            query: TextEdit::new("").expect("empty text"),
            apps: Apps::Unloaded,
            icons: icons::Icons::default(),
            loader: None,
            lookup: None,
            catalog_env: None,
            pinned_apps: Vec::new(),
            waker: None,
            launch_when_ready: false,
            page: 0,
            dock_page: 0,
            result_page: 0,
            all_apps: false,
            connected: false,
            size: [0.0; 2],
            dirty: true,
            pending: false,
            hide_after_action: false,
            request: None,
            error: None,
            surface_focused: true,
            background_pressed: false,
            pointer: None,
        }
    }
    /// Use an already-loaded catalog instead of scanning the XDG directories.
    pub fn with_catalog(mut self, catalog: Catalog) -> Self {
        self.apps = Apps::Ready(catalog);
        self
    }
    pub fn with_icon_lookup(mut self, lookup: IconLookup) -> Self {
        self.lookup = Some(lookup);
        self
    }
    /// Scan these directories instead of the process's XDG environment.
    pub fn with_catalog_environment(mut self, env: Environment) -> Self {
        self.catalog_env = Some(env);
        self
    }
    /// Desktop-file IDs shown before apps running in the active workspace.
    pub fn with_pinned_apps(mut self, pinned_apps: Vec<String>) -> Self {
        self.pinned_apps = pinned_apps;
        self
    }
    /// True while the worker still owes the UI a catalog or icons.
    pub fn is_loading(&self) -> bool {
        matches!(self.apps, Apps::Loading) || self.icons.is_loading()
    }
    pub fn browsed_workspace(&self) -> Option<WorkspaceId> {
        self.workspace
    }
    pub fn snapshot(&self) -> Option<&DesktopSnapshot> {
        self.snapshot.as_ref()
    }
    pub fn scene(&self) -> &Scene {
        &self.scene
    }
    fn window(&self, id: WindowId) -> Option<&WindowSummary> {
        self.snapshot.as_ref()?.windows.iter().find(|w| w.id == id)
    }
    fn minimized(&self, ws: WorkspaceId) -> Vec<&WindowSummary> {
        self.snapshot.as_ref().map_or_else(Vec::new, |s| {
            s.windows
                .iter()
                .filter(|w| w.workspace == ws && w.minimized)
                .collect()
        })
    }
    fn browse(&mut self, ws: WorkspaceId) {
        if !self.pending
            && self.workspace != Some(ws)
            && self
                .snapshot
                .as_ref()
                .is_some_and(|s| s.workspaces.iter().any(|w| w.workspace == ws))
        {
            self.dispatch(DesktopCommand::FocusWorkspace { workspace: ws });
            self.hide_after_action = false;
        }
    }

    fn adjacent(&mut self, next: bool) {
        let Some(s) = &self.snapshot else { return };
        let Some(index) = s
            .workspaces
            .iter()
            .position(|w| Some(w.workspace) == self.workspace)
        else {
            return;
        };
        let target = if next {
            index.saturating_add(1)
        } else {
            index.saturating_sub(1)
        };
        if let Some(w) = s.workspaces.get(target) {
            self.browse(w.workspace);
        }
    }
    fn dispatch(&mut self, command: DesktopCommand) {
        if !self.connected {
            self.error = Some("Desktop connection unavailable".into());
            self.dirty = true;
        } else if !self.pending {
            self.hide_after_action = !matches!(command, DesktopCommand::SetOverviewVisible { .. });
            self.request = Some(HostRequest::Desktop(command));
            self.pending = true;
            self.error = None;
            self.dirty = true;
        }
    }
    fn activate(&mut self, id: ElementId) {
        if self.pending {
            return;
        }
        match self.targets.get(&id).copied() {
            Some(Target::Workspace(ws)) => self.browse(ws),
            Some(Target::EnterWorkspace(ws)) => {
                self.dispatch(DesktopCommand::FocusWorkspace { workspace: ws })
            }
            Some(Target::StatusBar(knave_shell_ui::UiAction::FocusWorkspace(ws))) => {
                self.browse(ws)
            }
            Some(Target::StatusBar(knave_shell_ui::UiAction::OpenOverview)) => {
                // The overview is visible here, so the shared brand control hides it.
                if self.connected {
                    self.dispatch(DesktopCommand::SetOverviewVisible { visible: false });
                }
            }
            Some(Target::StatusBar(knave_shell_ui::UiAction::ToggleQuickSettings)) => {
                if self.connected {
                    self.quick_settings_after_hide = true;
                    self.dispatch(DesktopCommand::SetOverviewVisible { visible: false });
                }
            }
            Some(Target::StatusBar(knave_shell_ui::UiAction::ToggleNotifications)) => {
                if self.connected {
                    self.notifications_after_hide = true;
                    self.dispatch(DesktopCommand::SetOverviewVisible { visible: false });
                }
            }
            Some(Target::StatusBar(knave_shell_ui::UiAction::Applet(id))) => {
                self.request = Some(HostRequest::Applet(id));
            }
            Some(Target::StatusBar(_)) => {}
            Some(Target::Window(id)) => {
                if let Some(window) = self.window(id) {
                    self.dispatch(if window.minimized {
                        DesktopCommand::RestoreWindow { window: id }
                    } else {
                        DesktopCommand::FocusWindow { window: id }
                    });
                }
            }
            Some(Target::App(index)) => self.launch(index),
            Some(Target::AllApps) => {
                self.all_apps = !self.all_apps;
                self.result_page = 0;
                if self.all_apps {
                    self.clear_search();
                    self.ensure_catalog();
                } else {
                    self.clear_query();
                }
                self.dirty = true;
            }
            Some(Target::DockWindow(id)) => {
                if let Some(window) = self.window(id) {
                    self.dispatch(if window.minimized {
                        DesktopCommand::RestoreWindow { window: id }
                    } else {
                        DesktopCommand::FocusWindow { window: id }
                    });
                }
            }
            Some(Target::Page(next)) => {
                let page = if self.query.text().is_empty() && !self.all_apps {
                    &mut self.page
                } else {
                    &mut self.result_page
                };
                *page = if next {
                    page.saturating_add(1)
                } else {
                    page.saturating_sub(1)
                };
                self.dirty = true;
            }
            Some(Target::DockPage(next)) => {
                self.dock_page = if next {
                    self.dock_page.saturating_add(1)
                } else {
                    self.dock_page.saturating_sub(1)
                };
                self.dirty = true;
            }
            None => {}
        }
    }
    fn launch(&mut self, index: usize) {
        if let Apps::Ready(catalog) = &self.apps
            && let Some(app) = catalog.get(index)
        {
            let argv = app.argv.clone();
            self.dispatch(DesktopCommand::Spawn { argv });
        }
    }
    fn clear_query(&mut self) {
        self.clear_search();
        self.all_apps = false;
    }
    fn clear_search(&mut self) {
        let _ = self.scene.set_focus(None);
        self.query = TextEdit::new("").expect("empty text");
        self.scene
            .set_widget(SEARCH, Widget::TextInput(self.query.clone()))
            .expect("search input exists in the overview scene");
        self.result_page = 0;
        self.dirty = true;
    }
    fn reset_view(&mut self) {
        self.query = TextEdit::new("").expect("empty text");
        // Drop the old text widget and its captured input as well as the model,
        // so a late key release cannot copy the old query back into state.
        let user_theme_css = self.scene.user_theme_stylesheet().to_owned();
        let policy = self.scene.theme_policy();
        self.scene = Scene::new(Element::new(0, Widget::Panel)).expect("valid empty scene");
        self.scene
            .set_theme_policy_stylesheets(
                policy,
                knave_shell_ui::styles::STYLESHEET,
                &user_theme_css,
            )
            .expect("selected shell theme stylesheet is valid");
        self.targets.clear();
        self.panes.clear();
        self.workspace = None;
        self.page = 0;
        self.dock_page = 0;
        self.result_page = 0;
        self.all_apps = false;
        self.launch_when_ready = false;
        self.hide_after_action = false;
        self.background_pressed = false;
        self.pointer = None;
        self.error = None;
        self.dirty = true;
    }
    fn search_page_size(&self) -> usize {
        (((self.size[1]
            - STATUS_BAR_HEIGHT
            - 28.0f32.min(self.size[1] * 0.04)
            - 54.0f32.min(self.size[1] * 0.15)
            // Keep the result panel's gap and header above the bottom dock.
            - 12.0
            - 66.0
            - 80.0)
            / 60.0)
            .floor() as usize)
            .clamp(1, 8)
    }
    /// Validate the composition without a Wayland connection or GPU.
    pub fn check(&mut self, size: [f32; 2]) -> Result<(), UiError> {
        self.size = size;
        let mut scene = Scene::new(self.compose())?;
        scene
            .set_theme_policy_stylesheets(
                self.scene.theme_policy(),
                knave_shell_ui::styles::STYLESHEET,
                self.scene.user_theme_stylesheet(),
            )
            .expect("selected shell theme stylesheet is valid");
        Ok(())
    }
    fn loader(&mut self) -> &Loader {
        if self.loader.is_none() {
            let lookup = self.lookup.take().unwrap_or_else(IconLookup::from_process);
            self.loader = Some(Loader::start(lookup, self.waker.clone()));
        }
        self.loader.as_ref().expect("started above")
    }
    fn ensure_catalog(&mut self) {
        if matches!(self.apps, Apps::Unloaded) {
            let env = self
                .catalog_env
                .clone()
                .unwrap_or_else(Environment::from_process);
            self.apps = if self.loader().submit(Job::Catalog(env)) {
                Apps::Loading
            } else {
                Apps::Unavailable
            };
        }
    }
    /// Move finished worker results into UI state.
    fn absorb(&mut self) {
        let done = match &self.loader {
            Some(loader) => loader.take(),
            None => return,
        };
        for result in done {
            match result {
                Done::Catalog(Ok(catalog)) => {
                    if catalog.truncated() {
                        eprintln!("knave-shell: application catalog is incomplete (scan limit)");
                    }
                    self.apps = Apps::Ready(catalog);
                    self.result_page = 0;
                    if std::mem::take(&mut self.launch_when_ready)
                        && let Some(&index) = self.search().first()
                    {
                        self.launch(index);
                    }
                }
                Done::Catalog(Err(error)) => {
                    eprintln!("knave-shell: {error}");
                    self.apps = Apps::Unavailable;
                    self.launch_when_ready = false;
                }
                Done::Icon(name, image) => self.icons.finish(name, image),
            }
            self.dirty = true;
        }
    }
    /// Catalog indexes for the current query, best match first.
    fn search(&self) -> Vec<usize> {
        match &self.apps {
            Apps::Ready(catalog) if self.all_apps && self.query.text().trim().is_empty() => {
                (0..catalog.len()).collect()
            }
            Apps::Ready(catalog) => catalog.search(self.query.text(), MAX_RESULTS),
            Apps::Unloaded | Apps::Loading | Apps::Unavailable => Vec::new(),
        }
    }
    fn sync_query(&mut self) {
        if let Some(Widget::TextInput(edit)) = self.scene.widget(SEARCH) {
            let changed = self.query.text() != edit.text();
            self.query = edit.clone();
            if changed {
                self.launch_when_ready = false;
                if !self.query.text().is_empty() {
                    self.ensure_catalog();
                }
                self.result_page = 0;
                self.dirty = true;
            }
        }
    }
    fn rebuild(&mut self, text: &mut dyn TextMeasurer) {
        let focus = self.scene.focus();
        let root = self.compose();
        let user_theme_css = self.scene.user_theme_stylesheet().to_owned();
        let policy = self.scene.theme_policy();
        match Scene::new(root) {
            Ok(mut scene) => {
                if let Err(error) = scene.set_theme_policy_stylesheets(
                    policy,
                    knave_shell_ui::styles::STYLESHEET,
                    &user_theme_css,
                ) {
                    self.host_error(&format!("Could not apply appearance theme: {error}"));
                    return;
                }
                scene.layout(self.size, text);
                if !self.surface_focused {
                    scene.event(Input::FocusLost);
                }
                if let Some(id) = focus {
                    let _ = scene.set_focus(Some(id));
                }
                if scene.focus().is_none() {
                    let fallback = if !self.query.text().is_empty() {
                        Some(SEARCH)
                    } else {
                        self.targets.iter().find_map(|(id, t)| {
                            matches!(t, Target::EnterWorkspace(_)).then_some(*id)
                        })
                    };
                    let _ = scene.set_focus(fallback);
                }
                if let Some(p) = self.pointer {
                    scene.event(Input::PointerMove(p));
                }
                self.scene = scene;
            }
            Err(error) => {
                self.error = Some(error.to_string());
            }
        }
        self.dirty = false;
    }
}
impl Application for Overview {
    fn theme_policy_changed(
        &mut self,
        policy: knave_ui::toolkit::ThemePolicy,
        css: &str,
    ) -> Result<(), knave_ui::toolkit::StylesheetError> {
        self.scene
            .set_theme_policy_stylesheets(policy, knave_shell_ui::styles::STYLESHEET, css)?;
        self.theme = policy.scheme;
        self.dirty = true;
        Ok(())
    }
    fn applets(&mut self, applets: &[knave_shell_ui::status_bar::Applet]) {
        self.applets = applets.to_vec();
        self.dirty = true;
    }
    fn uses_status_bar(&self) -> bool {
        true
    }
    fn notifications_enabled(&mut self, enabled: bool) {
        self.notifications_enabled = enabled;
        self.dirty = true;
    }
    fn notification_snapshot(
        &mut self,
        snapshot: Option<&knave_desktop_api::notifications::NotificationSnapshot>,
    ) {
        self.notifications = snapshot.cloned();
        self.dirty = true;
    }
    fn uses_desktop(&self) -> bool {
        true
    }
    fn desktop_snapshot(&mut self, snapshot: &DesktopSnapshot) {
        if self.snapshot.as_ref() == Some(snapshot) && self.connected {
            return;
        }
        if snapshot.windows.len() > MAX_WINDOWS || snapshot.workspaces.len() > MAX_WORKSPACES {
            self.connected = false;
            // Cancel an activation not yet handed to the host against the rejected model.
            if matches!(self.request, Some(HostRequest::Desktop(_))) {
                self.request = None;
                self.pending = false;
            }
            self.host_error("Desktop exceeds the supported overview capacity");
            return;
        }
        if self.surface_visible() != snapshot.overview_visible {
            self.reset_view();
        }
        self.connected = true;
        self.error = None;
        if !snapshot.overview_visible {
            self.hide_after_action = false;
        }
        let previous = self.workspace;
        self.workspace = snapshot
            .workspaces
            .iter()
            .find(|w| w.active)
            .or(snapshot.workspaces.first())
            .map(|w| w.workspace);
        if self.workspace != previous {
            self.page = 0;
            self.dock_page = 0;
            let _ = self.scene.set_focus(None);
        }
        self.window_ids
            .retain(|id, _| snapshot.windows.iter().any(|w| w.id == *id));
        for w in &snapshot.windows {
            if !self.window_ids.contains_key(&w.id) {
                self.window_ids.insert(w.id, ElementId(self.next_id));
                self.next_id += 1;
            }
        }
        self.snapshot = Some(snapshot.clone());
        // Minimized icons resolve through the catalog; warm it up before any
        // query when the desktop already reports minimized windows. This is
        // event-driven here, never on the frame path.
        if matches!(self.apps, Apps::Unloaded) && snapshot.overview_visible {
            self.ensure_catalog();
        }
        self.dirty = true;
    }
    fn focused_app_icon(&mut self, app_id: Option<&str>, image: Option<&knave_ui::UiImage>) {
        if let Some(app_id) = app_id {
            let current_app = self.snapshot.as_ref().and_then(|snapshot| {
                let workspace = snapshot
                    .workspaces
                    .iter()
                    .find(|workspace| workspace.active)?;
                snapshot
                    .windows
                    .iter()
                    .find(|window| {
                        window.focused
                            && !window.minimized
                            && window.workspace == workspace.workspace
                    })
                    .map(|window| window.app_id.as_str())
            });
            if current_app != Some(app_id) {
                return;
            }
        }
        self.focused_icon = image.cloned();
        self.dirty = true;
    }
    fn desktop_unavailable(&mut self) {
        self.focused_icon = None;
        self.notifications_after_hide = false;
        self.quick_settings_after_hide = false;
        self.reset_view();
        self.connected = false;
        if self.request.take().is_some() {
            self.pending = false;
        }
        self.hide_after_action = false;
        self.host_error("Desktop connection unavailable — retrying");
    }
    fn overview_panes(&self) -> Option<Vec<OverviewPane>> {
        Some(self.panes.clone())
    }
    fn desktop_action_finished(&mut self, result: Result<(), String>) {
        if !self.pending {
            return;
        }
        self.pending = false;
        let notifications_after_hide = std::mem::take(&mut self.notifications_after_hide);
        let quick_settings_after_hide = std::mem::take(&mut self.quick_settings_after_hide);
        match result {
            Ok(()) if quick_settings_after_hide => {
                self.request = Some(HostRequest::ToggleQuickSettings);
            }
            Ok(()) if notifications_after_hide => {
                self.request = Some(HostRequest::ToggleNotifications);
            }
            Ok(()) if self.hide_after_action && self.surface_visible() => {
                self.hide_after_action = false;
                self.dispatch(DesktopCommand::SetOverviewVisible { visible: false });
            }
            Ok(()) => self.hide_after_action = false,
            Err(error) => self.error = Some(error),
        }
        self.dirty = true;
    }
    fn host_error(&mut self, message: &str) {
        self.error = Some(message.chars().take(256).collect());
        self.dirty = true;
    }
    fn cursor(&self) -> CursorShape {
        self.scene.cursor()
    }
    fn text_input(&self) -> Option<TextInputState> {
        self.scene.text_input_state()
    }
    fn take_request(&mut self) -> Option<HostRequest> {
        self.request.take()
    }
    fn should_close(&self) -> bool {
        false
    }
    fn surface_visible(&self) -> bool {
        self.connected
            && self
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.overview_visible)
    }
    fn needs_frame(&self) -> bool {
        self.dirty
            || self.scene.needs_frame()
            || self.loader.as_ref().is_some_and(Loader::has_results)
    }
    fn set_waker(&mut self, waker: knave_wayland::Waker) {
        self.waker = Some(waker);
    }
    fn input(&mut self, event: Input) {
        match &event {
            Input::FocusGained => self.surface_focused = true,
            Input::FocusLost => {
                self.surface_focused = false;
                self.background_pressed = false;
            }
            Input::PointerMove(p) => self.pointer = Some(*p),
            Input::PointerLeave => {
                self.pointer = None;
                self.background_pressed = false;
            }
            Input::PointerDown(p) => self.background_pressed = self.scene.hit_test(*p).is_none(),
            Input::PointerUp(p) => {
                if self.background_pressed && self.scene.hit_test(*p).is_none() {
                    if self.query.text().is_empty() {
                        self.dispatch(DesktopCommand::SetOverviewVisible { visible: false });
                    } else {
                        self.clear_query();
                    }
                }
                self.background_pressed = false;
            }
            Input::Key {
                key,
                pressed: true,
                repeat,
                modifiers,
            } => {
                if *key == Key::Escape && !repeat {
                    self.dispatch(DesktopCommand::SetOverviewVisible { visible: false });
                    return;
                }
                if *key == Key::K && modifiers.control {
                    let _ = self.scene.set_focus(Some(SEARCH));
                    return;
                }
                if matches!(key, Key::Left | Key::Right)
                    && (modifiers.alt
                        || (self.query.text().is_empty() && self.scene.focus() != Some(SEARCH)))
                {
                    self.adjacent(*key == Key::Right);
                    return;
                }
                if *key == Key::Enter && !repeat && self.scene.focus() == Some(SEARCH) {
                    if let Some(&index) = self
                        .search()
                        .get(self.result_page * self.search_page_size())
                    {
                        self.launch(index);
                    } else if matches!(self.apps, Apps::Loading) {
                        self.launch_when_ready = true;
                    }
                    return;
                }
                if *key == Key::Down
                    && self.scene.focus() == Some(SEARCH)
                    && !self.query.text().is_empty()
                {
                    if let Some(&index) = self
                        .search()
                        .get(self.result_page * self.search_page_size())
                    {
                        let _ = self.scene.set_focus(Some(app_element(index)));
                    }
                    return;
                }
            }
            Input::Text(value)
                if value.chars().any(|c| !c.is_control()) && self.surface_focused =>
            {
                let _ = self.scene.set_focus(Some(SEARCH));
            }
            _ => {}
        }
        let result = self.scene.event(event);
        self.sync_query();
        match result.action {
            Some(Action::Activated(id)) => self.activate(id),
            Some(Action::Copy(value) | Action::Cut { copied: value, .. }) => {
                if !self.pending {
                    self.request = Some(HostRequest::Copy(value));
                }
            }
            Some(Action::RequestPaste(id)) if !self.pending => {
                self.request = Some(HostRequest::Paste(id))
            }
            _ => {}
        }
    }
    fn frame(&mut self, size: [f32; 2], text: &mut dyn TextMeasurer) -> &DisplayList {
        self.absorb();
        if size != self.size {
            self.size = size;
            self.dirty = true;
        }
        if self.dirty {
            self.rebuild(text);
        }
        self.display = self.scene.display_list(size, text).clone();
        self.display
            .commands
            .extend(self.status_bar_component(size[0]).display.commands);
        self.display.clear_color = knave_ui::Color::rgba(0, 0, 0, 0);
        &self.display
    }
}

#[cfg(test)]
mod tests;
