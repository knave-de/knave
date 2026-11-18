use super::*;
use knave_ui::{Color, ImageFit, ImageStyle, Rect, ShapePaint, TextAlign, TextWrap, UiImage};

fn placed(mut e: Element, r: Rect) -> Element {
    e.layout = Layout {
        width: Length::Px(r.width.max(0.0)),
        height: Length::Px(r.height.max(0.0)),
        flow: Flow::Overlay,
        offset: [r.x, r.y],
        gap: 0.0,
        ..Default::default()
    };
    e
}
fn panel(id: u64, rect: Rect) -> Element {
    placed(Element::new(id, Widget::Panel), rect)
}
fn text(id: u64, value: impl Into<String>, r: Rect, size: f32, class: &str) -> Element {
    let mut e = placed(Element::new(id, Widget::Text(value.into())), r);
    e.style.text.font_size = size;
    e.style.text.line_height = size * 1.35;
    e.classes.push(class.into());
    e.style.text.wrap = TextWrap::None;
    e
}
fn control(id: u64, value: impl Into<String>, r: Rect) -> Element {
    let mut e = placed(
        Element::new(id, Widget::Button(value.into())).class("overview-control"),
        r,
    );
    // Shell CSS owns surface and interaction paint; layout stays in the composition.
    e.layout = e.layout.with_padding(Insets::symmetric(10.0, 14.0));
    e
}
fn short(value: &str, limit: usize) -> String {
    let mut s: String = value.chars().take(limit).collect();
    if value.chars().count() > limit {
        s.push('…');
    }
    s
}

/// One reserved element-id base keeps projected bar nodes apart from the
/// overview's own elements regardless of what the bar component produces.
const BAR_ELEMENT_ID_BASE: u64 = 0x800_0000_0000;
/// Derived per-tray-icon element id for a window's allocated control id.
const TRAY_ICON_ELEMENT_BASE: u64 = 0x500_0000_0000;
pub(super) const DOCK_ELEMENT_ID_BASE: u64 = 0x600_0000_0000;
const SEARCH_TEXT_ELEMENT_ID_BASE: u64 = 0x700_0000_0000;

struct DockItem {
    name: String,
    icon: String,
    catalog_index: Option<usize>,
    windows: Vec<WindowSummary>,
}

fn desktop_id_key(id: &str) -> String {
    id.strip_suffix(".desktop").unwrap_or(id).to_lowercase()
}

fn catalog_index(catalog: &Catalog, app_id: &str) -> Option<usize> {
    let key = desktop_id_key(app_id);
    let mut named = None;
    for index in 0..catalog.len() {
        let app = catalog.get(index)?;
        if desktop_id_key(&app.id) == key {
            return Some(index);
        }
        if named.is_none() && app.name.eq_ignore_ascii_case(app_id) {
            named = Some(index);
        }
    }
    named
}

impl Overview {
    pub(super) fn compose(&mut self) -> Element {
        self.targets.clear();
        self.panes.clear();
        self.pane_cards.clear();
        self.main_pane = None;
        let [width, height] = self.size;
        let mut root = Element::new(0, Widget::Panel).layout(Layout {
            height: Length::Fill,
            flow: Flow::Overlay,
            gap: 0.0,
            ..Default::default()
        });
        root = root.without_surface();
        root.style.background = Some(ShapePaint::fill(Color::rgba(0, 0, 0, 0)));
        let pad = (width * 0.045).clamp(12.0, 64.0);
        let button_width = 154.0f32.min((width - pad * 2.0).max(1.0) * 0.38);
        let gap = 10.0;
        let group_width = (width - pad * 2.0).min(720.0);
        let search_width = (group_width - button_width - gap).max(1.0);
        let search_rect = Rect::new(
            (width - group_width) / 2.0,
            STATUS_BAR_HEIGHT + 28.0f32.min(height * 0.04),
            search_width,
            54.0f32.min(height * 0.15),
        );
        let mut search = placed(
            Element::new(SEARCH.0, Widget::TextInput(self.query.clone())),
            search_rect,
        );
        search.layout = search.layout.with_padding(Insets {
            top: 10.0,
            right: 22.0,
            bottom: 10.0,
            left: 22.0,
        });
        search.classes.push("overview-search".into());
        search.style.text.font_size = 16.0;
        search.style.text.line_height = 22.0;
        if self.query.text().is_empty() {
            search.children.push(text(
                3,
                "Search applications…",
                Rect::new(22.0, 16.0, (search_width - 44.0).max(0.0), 22.0),
                16.0,
                "overview-muted",
            ));
        }
        // Child offsets are relative to the content area; search decorations use the border box.
        search.layout.clip = ClipMode::Bounds;
        // Padding applies to control text; overlay children are placed relative to that padding.
        for child in &mut search.children {
            child.layout.offset[0] -= 22.0;
            child.layout.offset[1] -= 10.0;
        }
        root.children.push(search);
        let mut all_apps = control(
            4,
            "All applications",
            Rect::new(
                search_rect.x + search_rect.width + gap,
                search_rect.y,
                button_width,
                search_rect.height,
            ),
        );
        all_apps.enabled = !self.pending;
        self.targets.insert(ElementId(4), Target::AllApps);
        root.children.push(all_apps);
        if self.query.text().is_empty()
            && !self.all_apps
            && let (Some(snapshot), Some(ws)) = (self.snapshot.clone(), self.workspace)
        {
            let strip_y = search_rect.y + search_rect.height + 22.0;
            let item_gap = 10.0;
            let capacity = self
                .workspace_strip_capacity()
                .min(snapshot.workspaces.len().max(1));
            let item_w: f32 = 132.0;
            let strip_h = (item_w * 9.0 / 16.0).min((height * 0.16).max(24.0));
            let workspaces = snapshot.workspaces.clone();
            let viewport_w =
                item_w * capacity as f32 + item_gap * capacity.saturating_sub(1) as f32;
            let viewport_x = (width - viewport_w) / 2.0;
            let viewport_id = ElementId(0x120_0000_0000);
            let mut strip = placed(
                Element::new(viewport_id.0, Widget::Panel),
                Rect::new(viewport_x, strip_y, viewport_w, strip_h),
            );
            strip.classes.push("overview-workspace-strip".into());
            strip.layout.clip = ClipMode::Bounds;
            strip.layout.flow = Flow::Row;
            strip.layout.gap = item_gap;
            for (index, summary) in workspaces
                .iter()
                .enumerate()
                .skip(self.workspace_scroll_offset)
                .take(capacity)
            {
                let workspace = summary.workspace;
                let rect = Rect::new(0.0, 0.0, item_w, strip_h);
                let id = 0x100_0000_0000 + u64::from(workspace.0) * 512 + index as u64 + 1;
                let mut card = self.workspace_card_with_preview(
                    workspace,
                    rect,
                    workspace == ws,
                    id,
                    Some(viewport_id),
                );
                card.classes.push("overview-workspace-thumbnail".into());
                let badge = placed(
                    Element::new(id + 3, Widget::Panel).class("overview-workspace-badge"),
                    Rect::new(7.0, (strip_h - 33.0).max(0.0), 24.0, 24.0),
                );
                card.children.push(badge);
                let mut label = text(
                    id + 2,
                    workspace.0.to_string(),
                    Rect::new(7.0, (strip_h - 33.0).max(0.0), 24.0, 24.0),
                    14.0,
                    "overview-workspace-number",
                );
                label.style.text.align = TextAlign::Center;
                label.style.text.weight = 600;
                card.children.push(label);
                strip.children.push(card);
            }
            root.children.push(strip);

            for (id, label, next, x) in [
                (6, "‹", false, viewport_x - 44.0),
                (7, "›", true, viewport_x + viewport_w + 8.0),
            ] {
                let mut arrow = control(
                    id,
                    label,
                    Rect::new(x, strip_y + (strip_h - 48.0) / 2.0, 36.0, 48.0),
                );
                arrow.style.text.font_size = 24.0;
                arrow.enabled = !self.pending;
                self.targets
                    .insert(ElementId(id), Target::WorkspaceStripPage(next));
                root.children.push(arrow);
            }

            let index = snapshot
                .workspaces
                .iter()
                .position(|w| w.workspace == ws)
                .unwrap_or(0);
            let top = strip_y + strip_h + 28.0;
            let shelf_room = if self.minimized(ws).is_empty() {
                0.0
            } else {
                54.0
            };
            let bottom_room = (140.0f32 + shelf_room).min(height * 0.24);
            let available_h = (height - top - bottom_room).max(40.0);
            let preview_width = (width * 0.80).min(available_h * 16.0 / 9.0).max(1.0);
            let preview_height = (preview_width * 9.0 / 16.0).min(available_h);
            let x = (width - preview_width) / 2.0;
            let y = top + (available_h - preview_height) / 2.0;
            let rect = Rect::new(x, y, preview_width, preview_height);
            let spaces: Vec<_> = snapshot
                .workspaces
                .iter()
                .enumerate()
                .filter(|(i, _)| i.abs_diff(index) <= 1)
                .map(|(i, w)| (i, w.clone()))
                .collect();
            for (i, w) in spaces.iter().filter(|(i, _)| *i != index) {
                let scale = 0.86;
                let small_w = preview_width * scale;
                let small_h = preview_height * scale;
                let side_x = if *i < index {
                    x - small_w - 24.0
                } else {
                    x + preview_width + 24.0
                };
                root.children.push(self.workspace_card_with_preview(
                    w.workspace,
                    Rect::new(
                        side_x,
                        y + (preview_height - small_h) / 2.0,
                        small_w,
                        small_h,
                    ),
                    false,
                    0x200_0000_0000 + u64::from(w.workspace.0),
                    None,
                ));
            }
            root.children.push(self.workspace_card_with_preview(
                ws,
                rect,
                true,
                0x210_0000_0000 + u64::from(ws.0),
                None,
            ));
            self.main_pane = self.panes.last().map(|pane| {
                Rect::new(
                    pane.x as f32,
                    pane.y as f32,
                    pane.width as f32,
                    pane.height as f32,
                )
            });
            let hovered = self.hovered_preview();
            self.hovered_window = hovered.map(|(id, _)| id);
            if let Some((id, toolbar)) = hovered
                && let Some(controls) = self.window_controls(id, toolbar)
            {
                root.children.push(controls);
            }
            if let Some(shelf) = self.minimized_shelf(ws, rect) {
                root.children.push(shelf);
            }
        } else if self.query.text().is_empty() && !self.all_apps {
            let message = if self.snapshot.is_some() {
                "No workspaces available"
            } else {
                "Connecting to desktop…"
            };
            let mut e = text(
                11,
                message,
                Rect::new(pad, height * 0.4, (width - pad * 2.0).max(0.0), 40.0),
                22.0,
                "overview-muted",
            );
            e.style.text.align = TextAlign::Center;
            root.children.push(e);
        }
        if !self.query.text().is_empty() || self.all_apps {
            let results_rect = Rect::new(
                search_rect.x,
                search_rect.y,
                group_width,
                search_rect.height,
            );
            root.children.push(self.search_results(results_rect));
        }
        root.children.push(self.dock(width, height));
        let footer = if self.pending && self.hide_after_action {
            Some("Opening…".to_owned())
        } else {
            self.error.as_deref().map(|error| short(error, 140))
        };
        if let Some(footer) = footer {
            let mut status = text(
                12,
                footer,
                Rect::new(
                    pad,
                    (height - 26.0).max(0.0),
                    (width - pad * 2.0).max(0.0),
                    22.0,
                ),
                12.0,
                if self.error.is_some() {
                    "overview-error"
                } else {
                    "overview-muted"
                },
            );
            status.style.text.align = TextAlign::Center;
            root.children.push(status);
        }
        // Drawn last so the status bar strip stays on top of overview content.
        root.children.push(self.status_bar(width));
        knave_shell_ui::styles::root(root.class("shell-overlay"), self.theme)
    }

    fn dock(&mut self, width: f32, height: f32) -> Element {
        let Some(workspace) = self.workspace else {
            return panel(DOCK_ELEMENT_ID_BASE, Rect::new(0.0, height, 0.0, 0.0));
        };
        let mut pin_keys = std::collections::HashSet::new();
        for pinned in &self.pinned_apps {
            let key = match &self.apps {
                Apps::Ready(catalog) => catalog_index(catalog, pinned)
                    .and_then(|index| catalog.get(index).map(|app| desktop_id_key(&app.id)))
                    .unwrap_or_else(|| desktop_id_key(pinned)),
                Apps::Unloaded | Apps::Loading | Apps::Unavailable => desktop_id_key(pinned),
            };
            pin_keys.insert(key);
        }
        let mut items = Vec::<DockItem>::new();
        let mut indexes = HashMap::<String, usize>::new();
        let mut pinned_windows = HashMap::<String, Vec<WindowSummary>>::new();
        if let Some(snapshot) = &self.snapshot {
            for window in snapshot.windows.iter().filter(|w| w.workspace == workspace) {
                let matched = match &self.apps {
                    Apps::Ready(catalog) => catalog_index(catalog, &window.app_id),
                    Apps::Unloaded | Apps::Loading | Apps::Unavailable => None,
                };
                let key = matched
                    .and_then(|index| match &self.apps {
                        Apps::Ready(catalog) => {
                            catalog.get(index).map(|app| desktop_id_key(&app.id))
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| desktop_id_key(&window.app_id));
                if pin_keys.contains(&key) {
                    pinned_windows.entry(key).or_default().push(window.clone());
                    continue;
                }
                let item_index = if let Some(&item_index) = indexes.get(&key) {
                    item_index
                } else {
                    let (name, icon) = match (&self.apps, matched) {
                        (Apps::Ready(catalog), Some(index)) => catalog.get(index).map_or_else(
                            || (window.app_id.clone(), icons::FALLBACK.into()),
                            |app| {
                                (
                                    app.name.clone(),
                                    catalog
                                        .running_icon(&window.app_id)
                                        .unwrap_or(icons::FALLBACK)
                                        .to_owned(),
                                )
                            },
                        ),
                        (Apps::Ready(catalog), None) => (
                            window.app_id.clone(),
                            catalog
                                .running_icon(&window.app_id)
                                .unwrap_or(icons::FALLBACK)
                                .to_owned(),
                        ),
                        _ => (window.app_id.clone(), icons::FALLBACK.into()),
                    };
                    let item_index = items.len();
                    indexes.insert(key.clone(), item_index);
                    items.push(DockItem {
                        name,
                        icon,
                        catalog_index: matched,
                        windows: Vec::new(),
                    });
                    item_index
                };
                items[item_index].windows.push(window.clone());
            }
        }
        let running_count = items.len();
        let mut added_pins = std::collections::HashSet::new();
        for pinned in &self.pinned_apps {
            let app = match &self.apps {
                Apps::Ready(catalog) => catalog_index(catalog, pinned)
                    .and_then(|index| catalog.get(index).map(|app| (index, app))),
                Apps::Unloaded | Apps::Loading | Apps::Unavailable => None,
            };
            let key = app
                .map(|(_, app)| desktop_id_key(&app.id))
                .unwrap_or_else(|| desktop_id_key(pinned));
            if !added_pins.insert(key.clone()) {
                continue;
            }
            let (name, icon, catalog_index) = app.map_or_else(
                || {
                    (
                        pinned.strip_suffix(".desktop").unwrap_or(pinned).to_owned(),
                        icons::FALLBACK.into(),
                        None,
                    )
                },
                |(index, app)| {
                    (
                        app.name.clone(),
                        app.icon.clone().unwrap_or_else(|| icons::FALLBACK.into()),
                        Some(index),
                    )
                },
            );
            let windows = pinned_windows.remove(&key).unwrap_or_default();
            indexes.insert(key, items.len());
            items.push(DockItem {
                name,
                icon,
                catalog_index,
                windows,
            });
        }
        let has_divider = running_count > 0 && running_count < items.len();

        if items.is_empty() {
            return panel(DOCK_ELEMENT_ID_BASE, Rect::new(0.0, height, 0.0, 0.0));
        }
        let icon_size = 42.0f32.min((height - STATUS_BAR_HEIGHT - 24.0).max(20.0));
        let item_width = 58.0;
        let gap = 4.0;
        let divider_allowance = if has_divider { 14.0 } else { 0.0 };
        let max_width = (width - 24.0).max(item_width);
        let plain_capacity = (((max_width - 20.0 - divider_allowance + gap) / (item_width + gap))
            .floor() as usize)
            .max(1);
        let paged = items.len() > plain_capacity;
        let side_controls = if paged { 58.0 } else { 0.0 };
        let capacity = (((max_width - 20.0 - side_controls - divider_allowance + gap)
            / (item_width + gap))
            .floor() as usize)
            .max(1)
            .min(items.len());
        let pages = items.len().saturating_sub(1) / capacity + 1;
        self.dock_page = self.dock_page.min(pages.saturating_sub(1));
        let first = self.dock_page * capacity;
        let shown = items.len().saturating_sub(first).min(capacity);
        let divider_on_page = running_count > first && running_count < first + shown;
        let divider_space = if divider_on_page { 14.0 } else { 0.0 };
        let dock_width = (20.0
            + side_controls
            + shown as f32 * item_width
            + shown.saturating_sub(1) as f32 * gap
            + divider_space)
            .min(width.max(1.0));
        let dock_height = 68.0;
        let x = ((width - dock_width) / 2.0).max(0.0);
        let y = (height - dock_height - 12.0).max(STATUS_BAR_HEIGHT);
        let mut dock = panel(
            DOCK_ELEMENT_ID_BASE,
            Rect::new(x, y, dock_width, dock_height),
        );
        dock.classes.push("overview-dock".into());
        dock.layout.clip = ClipMode::Bounds;
        // Child offsets start inside the dock border.
        let item_offset = if paged { 38.0 } else { 9.0 };
        if divider_on_page {
            let slot = running_count - first;
            let divider_x = item_offset + slot as f32 * (item_width + gap) + 6.5;
            let mut divider = panel(
                DOCK_ELEMENT_ID_BASE + 0x5000,
                Rect::new(divider_x, 16.0, 1.0, 36.0),
            );
            divider.classes.push("overview-dock-divider".into());
            divider.style.background = Some(ShapePaint::fill(Color::rgba(90, 96, 108, 255)));
            dock.children.push(divider);
        }
        if paged {
            for (slot, next) in [(0, false), (1, true)] {
                let id = ElementId(DOCK_ELEMENT_ID_BASE + 0x4000 + slot as u64);
                let mut button = control(
                    id.0,
                    if next { "›" } else { "‹" },
                    Rect::new(if next { dock_width - 31.0 } else { 3.0 }, 20.0, 28.0, 28.0),
                );
                button.enabled = if next {
                    first + capacity < items.len()
                } else {
                    self.dock_page > 0
                };
                self.targets.insert(id, Target::DockPage(next));
                dock.children.push(button);
            }
        }
        for (slot, item) in items.iter().skip(first).take(shown).enumerate() {
            if self.icons.needs_request(&item.icon)
                && self.loader().submit(loader::Job::Icon(item.icon.clone()))
            {
                self.icons.requested(&item.icon);
            }
            let id = ElementId(DOCK_ELEMENT_ID_BASE + 1 + slot as u64);
            let target = item
                .windows
                .iter()
                .find(|window| window.focused && !window.minimized)
                .or_else(|| item.windows.iter().find(|window| !window.minimized))
                .or_else(|| item.windows.first())
                .map(|window| Target::DockWindow(window.id))
                .or_else(|| item.catalog_index.map(Target::App));
            let mut button = placed(
                Element::new(id.0, Widget::Button(String::new())),
                Rect::new(
                    item_offset
                        + slot as f32 * (item_width + gap)
                        + if divider_on_page && first + slot >= running_count {
                            divider_space
                        } else {
                            0.0
                        },
                    4.0,
                    item_width,
                    58.0,
                ),
            );
            button.classes.push("overview-dock-app".into());
            if item.windows.is_empty() {
                button.classes.push("overview-dock-app-idle".into());
            }
            button.enabled = target.is_some() && !self.pending;
            button.tooltip = Some(if item.windows.len() > 1 {
                format!(
                    "{} · {} windows",
                    short(&item.name, 100),
                    item.windows.len()
                )
            } else {
                short(&item.name, 120)
            });
            if let Some(target) = target {
                self.targets.insert(id, target);
            }
            if let Some(image) = self.icons.get(&item.icon).cloned() {
                button.children.push(placed(
                    Element::new(
                        DOCK_ELEMENT_ID_BASE + 0x1000 + slot as u64,
                        Widget::Image(
                            image,
                            ImageStyle {
                                fit: ImageFit::Contain,
                                ..Default::default()
                            },
                        ),
                    ),
                    Rect::new(
                        (item_width - icon_size) / 2.0 - 1.0,
                        2.0,
                        icon_size,
                        icon_size,
                    ),
                ));
            } else {
                let mut fallback = text(
                    DOCK_ELEMENT_ID_BASE + 0x1000 + slot as u64,
                    "▣",
                    Rect::new(-1.0, 2.0, item_width, icon_size),
                    icon_size * 0.7,
                    "overview-muted",
                );
                fallback.style.text.align = TextAlign::Center;
                button.children.push(fallback);
            }
            if !item.windows.is_empty() {
                let mut indicator = panel(
                    DOCK_ELEMENT_ID_BASE + 0x2000 + slot as u64,
                    Rect::new((item_width - 14.0) / 2.0 - 1.0, 54.0, 14.0, 3.0),
                );
                indicator.classes.push("overview-dock-running".into());
                button.children.push(indicator);
            }
            if item.windows.len() > 1 {
                button.children.push(text(
                    DOCK_ELEMENT_ID_BASE + 0x3000 + slot as u64,
                    item.windows.len().to_string(),
                    Rect::new(item_width - 17.0, 1.0, 14.0, 14.0),
                    9.0,
                    "overview-dock-count",
                ));
            }
            dock.children.push(button);
        }
        dock
    }
    // Retained controls route the shared component's hit targets; all pixels
    // come directly from its display list in frame().
    fn status_bar(&mut self, width: f32) -> Element {
        let component = self.status_bar_component(width);
        let mut bar = placed(
            Element::new(BAR_ELEMENT_ID_BASE, Widget::Panel),
            Rect::new(0.0, 0.0, width, STATUS_BAR_HEIGHT),
        );
        bar = bar.without_surface();
        bar.style.background = Some(ShapePaint::fill(Color::rgba(0, 0, 0, 0)));
        for (index, (bounds, action)) in component.scene.hit_targets().enumerate() {
            let id = BAR_ELEMENT_ID_BASE + 10 + index as u64;
            self.targets
                .insert(ElementId(id), Target::StatusBar(action));
            let mut button = placed(Element::new(id, Widget::Button(String::new())), bounds);
            let transparent = Color::rgba(0, 0, 0, 0);
            button = button.without_surface();
            button.style.background = Some(ShapePaint::fill(transparent));
            if let knave_shell_ui::UiAction::Applet(id) = action {
                button.tooltip = self
                    .applets
                    .iter()
                    .find(|item| item.id == id)
                    .map(|item| item.title.clone());
            }
            button.enabled = !self.pending;
            bar.children.push(button);
        }
        bar
    }
    /// Icon theme name for a minimized window's application; apps missing
    /// from the catalog (or with no catalog at all) reuse the generic
    /// executable icon, with a built-in glyph if theme lookup also fails.
    fn window_icon_name(&self, app_id: &str) -> String {
        match &self.apps {
            Apps::Ready(catalog) => catalog.running_icon(app_id).map(str::to_owned),
            _ => None,
        }
        .unwrap_or_else(|| icons::FALLBACK.to_owned())
    }
    pub(super) fn hovered_preview(&self) -> Option<(WindowId, Rect)> {
        let point = self.pointer?;
        let pane = self.main_pane?;
        let layout = self.snapshot.as_ref()?.overview_layout.as_ref()?;
        if Some(layout.workspace) != self.workspace || layout.width <= 0 || layout.height <= 0 {
            return None;
        }
        let contains = |rect: Rect| {
            point[0] >= rect.x
                && point[0] < rect.x + rect.width
                && point[1] >= rect.y
                && point[1] < rect.y + rect.height
        };
        if !contains(pane) || pane.width < 132.0 || pane.height < 42.0 {
            return None;
        }
        // Match Villain's centered Zoom transform for the live pane.
        let scale = (pane.width / layout.width as f32).max(pane.height / layout.height as f32);
        let origin_x = pane.x + (pane.width - scale * layout.width as f32) / 2.0;
        let origin_y = pane.y + (pane.height - scale * layout.height as f32) / 2.0;
        let projected = layout.windows.iter().filter_map(|window| {
            let rect = Rect::new(
                origin_x + window.x as f32 * scale,
                origin_y + window.y as f32 * scale,
                window.width as f32 * scale,
                window.height as f32 * scale,
            )
            .intersection(pane)?;
            let toolbar = Rect::new(
                (rect.x + rect.width - 128.0).max(pane.x + 4.0),
                (rect.y + 6.0).min(pane.y + pane.height - 36.0),
                124.0,
                32.0,
            );
            Some((window.id, rect, toolbar))
        });
        // Keep the group reachable even when it extends past a narrow preview.
        projected
            .clone()
            .find(|(id, _, toolbar)| Some(*id) == self.hovered_window && contains(*toolbar))
            .or_else(|| projected.into_iter().find(|(_, rect, _)| contains(*rect)))
            .map(|(id, _, toolbar)| (id, toolbar))
    }

    fn window_controls(&mut self, id: WindowId, rect: Rect) -> Option<Element> {
        use OverviewWindowAction as Control;
        let window = self.window(id)?;
        let floating = window.floating;
        let maximized = window.maximized;
        let fullscreen = window.fullscreen;
        let base = 0x220_0000_0000;
        let mut group = panel(base, rect).class("overview-window-controls");
        for (index, (action, icon, label, enabled)) in [
            (
                Control::ToggleFloating,
                if floating {
                    "layout-board"
                } else {
                    "picture-in-picture"
                },
                if floating {
                    "Tile window"
                } else {
                    "Float window"
                },
                !fullscreen && !maximized,
            ),
            (Control::Minimize, "minus", "Minimize window", true),
            (
                Control::ToggleMaximize,
                if maximized { "squares" } else { "square" },
                if maximized {
                    "Restore window"
                } else {
                    "Maximize window"
                },
                !fullscreen,
            ),
            (Control::Close, "x", "Close window", true),
        ]
        .into_iter()
        .enumerate()
        {
            let button_id = base + 1 + index as u64 * 2;
            let icon =
                TablerIcon::named(icon, IconVariant::Outline).expect("bundled window control icon");
            let mut button = placed(
                Element::tabler_button(button_id, button_id + 1, icon, label),
                Rect::new(2.0 + index as f32 * 30.0, 2.0, 30.0, 28.0),
            )
            .class("overview-window-control");
            button.children[0].layout.width = Length::Px(19.0);
            button.children[0].layout.height = Length::Px(19.0);
            button.children[0].layout.offset = [5.5, 4.5];
            button.enabled = enabled && !self.pending;
            self.targets
                .insert(ElementId(button_id), Target::WindowControl(id, action));
            group.children.push(button);
        }
        Some(group)
    }

    fn workspace_card_with_preview(
        &mut self,
        ws: WorkspaceId,
        rect: Rect,
        selected: bool,
        id: u64,
        pane_viewport: Option<ElementId>,
    ) -> Element {
        let mut card = placed(Element::new(id, Widget::Button(String::new())), rect);
        card.classes.push(if selected {
            "overview-card-selected".into()
        } else {
            "overview-card-idle".into()
        });
        // The compositor supplies each workspace preview beneath this
        // transparent button; retain only its visible selection border.
        card.layout.clip = ClipMode::Bounds;
        self.targets.insert(
            ElementId(id),
            if selected {
                Target::EnterWorkspace(ws)
            } else {
                Target::Workspace(ws)
            },
        );
        // The compositor fills the card; shell borders and controls remain above it.
        if let Some(viewport) = pane_viewport {
            self.pane_cards.push((ElementId(id), ws, viewport));
        } else if rect.height > 4.0
            && rect.width > 4.0
            && rect.x + rect.width > 0.0
            && rect.x < self.size[0]
        {
            self.panes.push(PaneGeometry {
                workspace: ws,
                x: (rect.x + 2.0).round() as i32,
                y: (rect.y + 2.0).round() as i32,
                width: (rect.width - 4.0).round() as u32,
                height: (rect.height - 4.0).round() as u32,
                card: ElementId(id),
            });
        }

        card
    }
    fn minimized_shelf(&mut self, ws: WorkspaceId, rect: Rect) -> Option<Element> {
        let min: Vec<_> = self.minimized(ws).into_iter().cloned().collect();
        if min.is_empty() {
            return None;
        }
        let id = 0x100_0000_0000 + u64::from(ws.0) * 32;
        let tray_h = 42.0f32.min(rect.height * 0.35);
        let icon_size = (tray_h - 10.0).max(16.0);
        let pitch = icon_size + 4.0;
        let max_width = (rect.width - 24.0).max(0.0);
        // Room for two paging controls when several pages exist.
        let paging_w = 2.0 * 28.0 + 6.0;
        let leading_w = 64.0;
        let capacity =
            (((max_width - leading_w - paging_w) / pitch).floor() as usize).clamp(1, PAGE_SIZE);
        self.page = self.page.min(min.len().saturating_sub(1) / capacity);
        let first = self.page * capacity;
        let shown = min.len().saturating_sub(first).min(capacity);
        let span = (shown as f32 - 1.0) * pitch + icon_size;
        let tray_w = if min.len() > capacity {
            leading_w + span + paging_w
        } else {
            leading_w + span
        }
        .min(max_width);
        let mut tray = panel(
            id + 3,
            Rect::new(rect.x + 12.0, rect.y + rect.height + 12.0, tray_w, tray_h),
        );
        tray.classes.push("overview-tray".into());
        tray.style.background = Some(ShapePaint::fill(Color::rgba(0, 0, 0, 0)));
        tray.layout.clip = ClipMode::Bounds;
        tray.children.push(text(
            id + 4,
            format!("▣ {}", min.len()),
            Rect::new(4.0, 5.0, 42.0, 30.0),
            13.0,
            "overview-muted",
        ));
        let mut divider = panel(id + 5, Rect::new(51.0, 6.0, 1.0, 30.0));
        divider.classes.push("overview-tray-divider".into());
        tray.children.push(divider);
        for (n, window) in min.iter().skip(first).take(shown).enumerate() {
            let wid = self.window_ids[&window.id];
            let title = if window.title.is_empty() {
                window.app_id.as_str()
            } else {
                window.title.as_str()
            };
            let icon_name = self.window_icon_name(&window.app_id);
            // Failed submits retry on the next frame, which the worker's results trigger.
            if self.icons.needs_request(&icon_name)
                && self.loader().submit(loader::Job::Icon(icon_name.clone()))
            {
                self.icons.requested(&icon_name);
            }
            let mut icon = placed(
                Element::new(wid.0, Widget::Button(String::new())),
                Rect::new(60.0 + n as f32 * pitch, 5.0, icon_size, icon_size),
            );
            icon.style.background = Some(ShapePaint::fill(Color::rgba(0, 0, 0, 0)));
            icon.classes.push("overview-window-icon".into());
            // Hover and accessibility name the exact window, not its application.
            icon.tooltip = Some(format!("Restore {}", short(title, 120)));
            icon.enabled = !self.pending;
            if let Some(image) = self.icons.get(&icon_name).cloned() {
                let side = (icon_size - 8.0).max(10.0);
                let inset = (icon_size - side) / 2.0;
                icon.children.push(placed(
                    Element::new(
                        TRAY_ICON_ELEMENT_BASE + wid.0,
                        Widget::Image(
                            image,
                            ImageStyle {
                                fit: ImageFit::Contain,
                                ..Default::default()
                            },
                        ),
                    ),
                    Rect::new(inset, inset, side, side),
                ));
            } else {
                let mut fallback = text(
                    TRAY_ICON_ELEMENT_BASE + wid.0,
                    "▣",
                    Rect::new(0.0, 0.0, icon_size, icon_size),
                    icon_size * 0.72,
                    "overview-muted",
                );
                fallback.style.text.align = TextAlign::Center;
                icon.children.push(fallback);
            }
            self.targets.insert(wid, Target::Window(window.id));
            tray.children.push(icon);
        }
        if min.len() > capacity {
            for (bid, label, next, bx) in [(30, "‹", false, 6.0), (31, "›", true, 34.0)] {
                let mut b = control(
                    bid,
                    label,
                    Rect::new(
                        (tray_w - 6.0 - paging_w + bx).max(0.0),
                        (tray_h - 23.0) / 2.0,
                        28.0,
                        23.0,
                    ),
                );
                b.layout.padding_edges = Some(Insets::symmetric(1.0, 8.0));
                b.enabled = if next {
                    first + capacity < min.len()
                } else {
                    self.page > 0
                };
                self.targets.insert(ElementId(bid), Target::Page(next));
                tray.children.push(b);
            }
        }
        Some(tray)
    }
    fn search_results(&mut self, search: Rect) -> Element {
        let matches = self.search();
        let page_size = self.search_page_size();
        self.result_page = self
            .result_page
            .min(matches.len().saturating_sub(1) / page_size);
        let start = self.result_page * page_size;
        let shown = matches.len().saturating_sub(start).min(page_size);
        let visible: Vec<(usize, String, String, String)> = match &self.apps {
            Apps::Ready(catalog) => matches
                .iter()
                .skip(start)
                .take(shown)
                .filter_map(|&index| {
                    let app = catalog.get(index)?;
                    let icon = app.icon.clone().unwrap_or_else(|| icons::FALLBACK.into());
                    Some((
                        index,
                        short(&app.name, 70),
                        short(&app.description, 96),
                        icon,
                    ))
                })
                .collect(),
            Apps::Unloaded | Apps::Loading | Apps::Unavailable => Vec::new(),
        };
        for (_, _, _, icon) in &visible {
            // Failed submits retry on the next frame, which the worker's results trigger.
            if self.icons.needs_request(icon)
                && self.loader().submit(loader::Job::Icon(icon.clone()))
            {
                self.icons.requested(icon);
            }
        }
        let rows: Vec<(usize, String, String, Option<UiImage>)> = visible
            .into_iter()
            .map(|(index, name, description, icon)| {
                (index, name, description, self.icons.get(&icon).cloned())
            })
            .collect();
        const CATALOG_HEADER: f32 = 62.0;
        const CATALOG_FOOTER: f32 = 48.0;
        let panel_height = if rows.is_empty() {
            CATALOG_HEADER + 48.0 + CATALOG_FOOTER
        } else {
            CATALOG_HEADER + rows.len() as f32 * 60.0 + CATALOG_FOOTER
        };
        let mut panel = panel(
            40,
            Rect::new(
                search.x,
                search.y + search.height + 12.0,
                search.width,
                panel_height,
            ),
        );
        panel.classes.push("overview-results".into());
        panel.layout.clip = ClipMode::Bounds;
        let mut heading = placed(
            Element::new(47, Widget::Panel).class("catalog-heading"),
            Rect::new(-1.0, -1.0, search.width + 2.0, CATALOG_HEADER),
        );
        let title = if self.query.text().is_empty() {
            "Applications"
        } else {
            "Search results"
        };
        heading.children.push(text(
            48,
            title,
            Rect::new(16.0, 8.0, (search.width - 32.0).max(0.0), 20.0),
            14.0,
            "catalog-title",
        ));
        let count = match matches.len() {
            1 => "1 application".to_owned(),
            n => format!("{n} applications"),
        };
        heading.children.push(text(
            49,
            count,
            Rect::new(16.0, 32.0, (search.width - 32.0).max(0.0), 16.0),
            11.0,
            "overview-muted",
        ));
        panel.children.push(heading);
        if !rows.is_empty() {
            panel.children.push(placed(
                Element::new(45, Widget::Panel).class("catalog-results-inset"),
                Rect::new(
                    7.0,
                    CATALOG_HEADER - 1.0,
                    (search.width - 16.0).max(0.0),
                    (rows.len() as f32 * 60.0).max(1.0),
                ),
            ));
        }
        for (n, (index, name, description, image)) in rows.into_iter().enumerate() {
            let id = app_element(index);
            let row = Rect::new(
                7.0,
                CATALOG_HEADER - 1.0 + n as f32 * 60.0,
                (search.width - 16.0).max(0.0),
                60.0,
            );
            let mut result = control(id.0, String::new(), row);
            let pad = Insets {
                top: 10.0,
                right: 14.0,
                bottom: 10.0,
                left: 66.0,
            };
            result.layout = result.layout.with_padding(pad);
            result.layout.clip = ClipMode::Bounds;
            result.semantics.name = Some(name.clone());
            if !description.is_empty() {
                result.semantics.description = Some(description.clone());
            }
            if let Some(image) = image {
                let icon = placed(
                    Element::new(
                        0x400_0000_0000 + index as u64,
                        Widget::Image(
                            image,
                            ImageStyle {
                                fit: ImageFit::Contain,
                                ..Default::default()
                            },
                        ),
                    ),
                    Rect::new(14.0 - pad.left, 10.0 - pad.top, 40.0, 40.0),
                );
                result.children.push(icon);
            }
            let name_id = SEARCH_TEXT_ELEMENT_ID_BASE + index as u64 * 2;
            let description_id = name_id + 1;
            let mut name_text = placed(
                Element::new(name_id, Widget::Text(name)),
                Rect::new(0.0, 0.0, (row.width - pad.left - pad.right).max(0.0), 18.0),
            );
            name_text.style.text.font_size = 13.0;
            name_text.style.text.line_height = 18.0;
            name_text.style.text.align = TextAlign::Start;
            name_text.style.text.weight = 600;
            result.children.push(name_text);
            if !description.is_empty() {
                let mut description_text = text(
                    description_id,
                    description,
                    Rect::new(0.0, 20.0, (row.width - pad.left - pad.right).max(0.0), 16.0),
                    11.0,
                    "overview-muted",
                );
                description_text.style.text.align = TextAlign::Start;
                result.children.push(description_text);
            }
            result.enabled = !self.pending;
            self.targets.insert(id, Target::App(index));
            panel.children.push(result);
        }
        if matches.is_empty() {
            panel.children.push(text(
                42,
                match self.apps {
                    Apps::Unavailable => "Application list unavailable",
                    Apps::Unloaded | Apps::Loading => "Loading applications…",
                    Apps::Ready(_) => "No applications found",
                },
                Rect::new(
                    18.0,
                    CATALOG_HEADER + 12.0,
                    (search.width - 36.0).max(0.0),
                    24.0,
                ),
                13.0,
                "overview-muted",
            ));
        }
        let footer_y = (panel_height - CATALOG_FOOTER).max(0.0);
        let footer = placed(
            Element::new(43, Widget::Panel).class("catalog-footer"),
            Rect::new(-1.0, footer_y - 1.0, search.width + 2.0, CATALOG_FOOTER),
        );
        panel.children.push(footer);
        panel.children.push(text(
            46,
            "Choose an application to open",
            Rect::new(16.0, footer_y + 14.0, (search.width - 152.0).max(0.0), 20.0),
            11.0,
            "overview-muted",
        ));
        let page_count = matches.len().div_ceil(page_size).max(1);
        panel.children.push(text(
            44,
            format!("{} / {page_count}", self.result_page + 1),
            Rect::new((search.width - 60.0).max(0.0), footer_y + 14.0, 28.0, 20.0),
            11.0,
            "overview-muted",
        ));
        for (id, label, next, x) in [
            (30, "‹", false, search.width - 88.0),
            (31, "›", true, search.width - 32.0),
        ] {
            let mut b = control(id, label, Rect::new(x, footer_y + 10.0, 28.0, 28.0));
            b.classes.push("catalog-footer-control".into());
            b.layout.padding_edges = Some(Insets::symmetric(2.0, 8.0));
            b.enabled = if next {
                start + page_size < matches.len()
            } else {
                self.result_page > 0
            };
            self.targets.insert(ElementId(id), Target::Page(next));
            panel.children.push(b);
        }
        panel
    }
}
