//! Complete status bar presentation shared by desktop and overview hosts.
use crate::{DisplayList, UiScene};
use knave_desktop_api::{DesktopSnapshot, notifications::NotificationSnapshot};
use knave_ui::{
    Color, Rect, TextStyle,
    toolkit::{
        ButtonVariant, Element, ElementId, Flow, Layout, Length, Scene as ToolkitScene, TextLayout,
        TextMeasurer, Widget,
    },
};

pub const STATUS_BAR_HEIGHT: f32 = 36.0;

const EDGE: f32 = 8.0;
const ISLAND_Y: f32 = 4.0;
const ISLAND_HEIGHT: f32 = 28.0;
const GROUP_GAP: f32 = 6.0;
const WORKSPACE_WIDTH: f32 = 22.0;
const WORKSPACE_GAP: f32 = 3.0;
const APPLET_PITCH: f32 = 25.0;
const APPLET_WIDTH: f32 = 22.0;
const SYSTEM_WIDTH: f32 = 48.0;
const BELL_WIDTH: f32 = 28.0;

/// Presentation data only; D-Bus endpoints remain owned by the tray worker.
#[derive(Clone, Debug, PartialEq)]
pub struct Applet {
    pub id: u64,
    pub title: String,
    pub image: Option<knave_ui::UiImage>,
    pub attention: bool,
}

/// Applet hit rectangles are also used by the runtime for pointer and popover placement.
pub fn applet_targets(
    width: f32,
    applets: &[Applet],
) -> impl Iterator<Item = (knave_ui::Rect, &Applet)> {
    applet_targets_with_notifications(width, applets, true)
}

/// Variant used by the bar runtime when notification support is unavailable.
pub fn applet_targets_with_notifications(
    width: f32,
    applets: &[Applet],
    notifications_enabled: bool,
) -> impl Iterator<Item = (knave_ui::Rect, &Applet)> {
    let bell_x = width - EDGE - SYSTEM_WIDTH - GROUP_GAP - BELL_WIDTH;
    let bell_visible = notifications_enabled && bell_x >= EDGE + 28.0 + GROUP_GAP;
    let bell_width = if bell_visible {
        BELL_WIDTH + GROUP_GAP
    } else {
        0.0
    };
    let bell_x = width - EDGE - SYSTEM_WIDTH - bell_width;
    let right = bell_x - GROUP_GAP - 4.0;
    let left_limit = EDGE + 28.0 + GROUP_GAP + 8.0 + WORKSPACE_WIDTH + GROUP_GAP + 4.0;
    let capacity = (((right - left_limit + WORKSPACE_GAP) / APPLET_PITCH)
        .floor()
        .max(0.0)) as usize;
    let count = capacity.min(applets.len());
    let start = right - count as f32 * APPLET_PITCH + WORKSPACE_GAP;
    applets
        .iter()
        .take(count)
        .enumerate()
        .map(move |(index, applet)| {
            (
                knave_ui::Rect::new(
                    start + index as f32 * APPLET_PITCH,
                    ISLAND_Y,
                    APPLET_WIDTH,
                    ISLAND_HEIGHT,
                ),
                applet,
            )
        })
}

pub struct StatusBar {
    pub scene: UiScene,
    pub display: DisplayList,
    toolkit_scene: ToolkitScene,
    toolkit_root: Element,
    size: [f32; 2],
    tinted_status_icons: Option<([Color; 3], [Option<knave_ui::UiImage>; 3])>,
}

impl StatusBar {
    /// Apply the selected bundled toolkit palette to the widgets in this bar.
    pub fn set_theme(
        &mut self,
        theme: knave_ui::toolkit::Theme,
    ) -> Result<(), knave_ui::toolkit::StylesheetError> {
        let mut policy = self.scene.theme_policy();
        policy.scheme = theme;
        let css = self.toolkit_scene.user_theme_stylesheet().to_owned();
        self.set_user_policy(policy, &css)
    }

    /// Apply the shell theme and user theme CSS while retaining any bar-level overrides.
    pub fn set_user_theme(
        &mut self,
        theme: knave_ui::toolkit::Theme,
        css: &str,
    ) -> Result<(), knave_ui::toolkit::StylesheetError> {
        self.set_user_policy(
            knave_ui::toolkit::ThemePolicy::new(
                theme,
                knave_ui::toolkit::Contrast::Standard,
                knave_ui::toolkit::Chroma::FullColor,
            ),
            css,
        )
    }

    pub fn set_user_policy(
        &mut self,
        policy: knave_ui::toolkit::ThemePolicy,
        css: &str,
    ) -> Result<(), knave_ui::toolkit::StylesheetError> {
        self.toolkit_scene
            .replace(crate::styles::root(
                self.toolkit_root.clone(),
                policy.scheme,
            ))
            .expect("status bar element identities are unique");
        self.toolkit_scene
            .set_theme_policy_stylesheets(policy, crate::styles::STYLESHEET, css)?;
        self.scene.set_theme_policy(policy);
        self.refresh_toolkit_output();
        Ok(())
    }

    /// Apply a replacement stylesheet to the standard toolkit widgets in this bar.
    pub fn set_stylesheet(
        &mut self,
        source: &str,
    ) -> Result<(), knave_ui::toolkit::StylesheetError> {
        self.toolkit_scene.set_stylesheet(source)?;
        self.refresh_toolkit_output();
        Ok(())
    }

    fn refresh_toolkit_output(&mut self) {
        let mut metrics = StatusTextMetrics;
        self.toolkit_scene.layout(self.size, &mut metrics);
        for (index, target) in self.scene.targets.iter_mut().enumerate() {
            if let Some(bounds) = self.toolkit_scene.bounds(ElementId(50_000 + index as u64)) {
                target.bounds = bounds;
            }
        }
        self.display = self
            .toolkit_scene
            .display_list(self.size, &mut metrics)
            .clone();
        let icon_colors = [40_000, 40_001, 40_002].map(|id| {
            self.toolkit_scene
                .computed_style(ElementId(60_000 + id))
                .map_or(Color::TEXT, |style| style.style.text.color)
        });
        if self
            .tinted_status_icons
            .as_ref()
            .is_none_or(|(cached_colors, _)| *cached_colors != icon_colors)
        {
            self.tinted_status_icons = Some((
                icon_colors,
                [
                    crate::status_icons::bell()
                        .map(|image| crate::status_icons::tint(&image, icon_colors[0])),
                    crate::status_icons::wifi()
                        .map(|image| crate::status_icons::tint(&image, icon_colors[1])),
                    crate::status_icons::volume()
                        .map(|image| crate::status_icons::tint(&image, icon_colors[2])),
                ],
            ));
        }
        let originals = [
            crate::status_icons::bell(),
            crate::status_icons::wifi(),
            crate::status_icons::volume(),
        ];
        let brand_source = crate::brand::image();
        let themed_brand = crate::brand::image_for(self.scene.theme_policy());
        let (_, tinted) = self
            .tinted_status_icons
            .as_ref()
            .expect("icon tint initialized");
        for command in &mut self.display.commands {
            let knave_ui::DisplayCommand::Image { image, .. } = command else {
                continue;
            };
            if let Some((_, replacement)) = originals.iter().zip(tinted).find(|(original, _)| {
                original
                    .as_ref()
                    .is_some_and(|source| source.cache_key() == image.cache_key())
            }) && let Some(replacement) = replacement
            {
                *image = replacement.clone();
            } else if brand_source
                .as_ref()
                .is_some_and(|source| source.cache_key() == image.cache_key())
                && let Some(replacement) = &themed_brand
            {
                *image = replacement.clone();
            }
        }
        self.display.revision = self.scene.revision();
    }
}

impl StatusBar {
    pub fn new(
        revision: u64,
        width: f32,
        height: f32,
        desktop: Option<&DesktopSnapshot>,
        notifications_enabled: bool,
        notifications: Option<&NotificationSnapshot>,
        applets: &[Applet],
    ) -> Self {
        Self::with_focused_icon(
            revision,
            width,
            height,
            desktop,
            notifications_enabled,
            notifications,
            applets,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_focused_icon(
        revision: u64,
        width: f32,
        height: f32,
        desktop: Option<&DesktopSnapshot>,
        notifications_enabled: bool,
        notifications: Option<&NotificationSnapshot>,
        applets: &[Applet],
        focused_icon: Option<&knave_ui::UiImage>,
    ) -> Self {
        let mut scene = UiScene::bar(revision, width, height);
        scene.targets.clear();
        scene.push(crate::UiNode::Panel {
            id: crate::NodeId(4_000),
            bounds: knave_ui::Rect::new(EDGE, ISLAND_Y, 28.0, ISLAND_HEIGHT),
            color: Color::rgba(0, 0, 0, 0),
        });
        scene.target(
            knave_ui::Rect::new(EDGE, ISLAND_Y, 28.0, ISLAND_HEIGHT),
            crate::UiAction::OpenOverview,
        );

        let applets: Vec<_> =
            applet_targets_with_notifications(width, applets, notifications_enabled).collect();
        let system_x = width - EDGE - SYSTEM_WIDTH;
        let bell_x = system_x - GROUP_GAP - BELL_WIDTH;
        let workspace_start = EDGE + 28.0 + GROUP_GAP;
        let bell_visible = notifications_enabled && bell_x >= EDGE + 28.0 + GROUP_GAP;
        let controls_left = if bell_visible {
            bell_x - GROUP_GAP
        } else {
            system_x - GROUP_GAP
        };
        let workspace_limit = applets
            .first()
            .map_or(controls_left, |(rect, _)| rect.x - 4.0 - GROUP_GAP)
            .min(controls_left);

        let workspaces = visible_workspaces(desktop, workspace_start, workspace_limit);
        if !workspaces.is_empty() {
            let group_x = workspace_start;
            for (index, workspace) in workspaces.iter().enumerate() {
                let x = group_x + 4.0 + index as f32 * (WORKSPACE_WIDTH + WORKSPACE_GAP);
                let bounds = knave_ui::Rect::new(x, ISLAND_Y + 3.0, WORKSPACE_WIDTH, 22.0);
                scene.push(crate::UiNode::Label {
                    id: crate::NodeId(10 + u64::from(workspace.workspace.0)),
                    bounds,
                    color: Color::TEXT,
                    text: if workspace.workspace.0 == 10 {
                        "0".into()
                    } else {
                        workspace.workspace.0.to_string()
                    },
                });
                scene.target(bounds, crate::UiAction::FocusWorkspace(workspace.workspace));
            }
        }

        let workspace_group_width = if workspaces.is_empty() {
            0.0
        } else {
            8.0 + workspaces.len() as f32 * WORKSPACE_WIDTH
                + workspaces.len().saturating_sub(1) as f32 * WORKSPACE_GAP
        };
        if !workspaces.is_empty() {
            scene.push(crate::UiNode::Panel {
                id: crate::NodeId(4_999),
                bounds: knave_ui::Rect::new(
                    workspace_start,
                    ISLAND_Y,
                    workspace_group_width,
                    ISLAND_HEIGHT,
                ),
                color: Color::rgba(0, 0, 0, 0),
            });
        }
        let focus_x = workspace_start
            + workspace_group_width
            + if workspaces.is_empty() {
                0.0
            } else {
                GROUP_GAP
            };
        let focus_right = applets
            .first()
            .map_or(controls_left, |(rect, _)| rect.x - 4.0 - GROUP_GAP);
        let focus = desktop.and_then(focused_window);
        let focus_available = focus_right - focus_x;
        // Focus is the first optional section to give way to workspace and control targets.
        if let Some(window) = focus.filter(|window| {
            focus_available >= 34.0 && (!window.title.is_empty() || focused_icon.is_some())
        }) {
            let icon_bounds = knave_ui::Rect::new(focus_x + 6.0, 10.0, 16.0, 16.0);
            if let Some(image) = focused_icon {
                scene.push(crate::UiNode::Image {
                    id: crate::NodeId(9),
                    bounds: icon_bounds,
                    image: image.clone(),
                });
            }
            let content_start = focus_x + 6.0;
            let text_x = if focused_icon.is_some() {
                icon_bounds.x + icon_bounds.width + 7.0
            } else {
                content_start
            };
            let max_text_width = if focused_icon.is_some() { 203.0 } else { 226.0 };
            let text_width = (focus_right - text_x - 6.0).clamp(0.0, max_text_width);
            if !window.title.is_empty() && text_width > 0.0 {
                let text = window.title.replace(['\n', '\r'], " ");
                if !text.is_empty() {
                    let rendered_width = text_width;
                    scene.push(crate::UiNode::Label {
                        id: crate::NodeId(8),
                        bounds: knave_ui::Rect::new(text_x, 0.0, rendered_width, STATUS_BAR_HEIGHT),
                        color: knave_ui::Color::TEXT,
                        text,
                    });
                }
            }
            let content_right = if !window.title.is_empty() && text_width > 0.0 {
                text_x + text_width
            } else if focused_icon.is_some() {
                icon_bounds.x + icon_bounds.width
            } else {
                content_start
            };
            let island_width = (content_right + 6.0 - focus_x)
                .max(28.0)
                .min(focus_right - focus_x);
            if island_width >= 28.0 {
                scene.push(crate::UiNode::Panel {
                    id: crate::NodeId(7_000),
                    bounds: knave_ui::Rect::new(focus_x, ISLAND_Y, island_width, ISLAND_HEIGHT),
                    color: Color::rgba(0, 0, 0, 0),
                });
            }
        }

        if !applets.is_empty() {
            let first = applets.first().unwrap().0;
            let last = applets.last().unwrap().0;
            scene.push(crate::UiNode::Panel {
                id: crate::NodeId(6_000),
                bounds: knave_ui::Rect::new(
                    first.x - 4.0,
                    ISLAND_Y,
                    last.x + last.width - first.x + 8.0,
                    ISLAND_HEIGHT,
                ),
                color: Color::rgba(0, 0, 0, 0),
            });
            for (bounds, item) in &applets {
                scene.target(*bounds, crate::UiAction::Applet(item.id));
                if let Some(image) = &item.image {
                    scene.push(crate::UiNode::Image {
                        id: crate::NodeId(20_000 + item.id),
                        bounds: knave_ui::Rect::new(bounds.x + 3.0, 10.0, 16.0, 16.0),
                        image: image.clone(),
                    });
                } else {
                    scene.push(crate::UiNode::Label {
                        id: crate::NodeId(20_000 + item.id),
                        bounds: knave_ui::Rect::new(bounds.x + 3.0, 6.0, 16.0, 24.0),
                        color: knave_ui::Color::TEXT,
                        text: item.title.chars().next().unwrap_or('?').to_string(),
                    });
                }
                if item.attention {
                    scene.push(crate::UiNode::Panel {
                        id: crate::NodeId(30_000 + item.id),
                        bounds: knave_ui::Rect::new(bounds.x + 7.0, 29.0, 8.0, 2.0),
                        color: knave_ui::Color::ACCENT,
                    });
                }
            }
        }

        let controls_visible = system_x >= EDGE + 28.0 + GROUP_GAP;
        if controls_visible {
            scene.target(
                knave_ui::Rect::new(system_x, ISLAND_Y, SYSTEM_WIDTH, ISLAND_HEIGHT),
                crate::UiAction::ToggleQuickSettings,
            );
        }
        if bell_visible {
            scene.target(
                knave_ui::Rect::new(bell_x, ISLAND_Y, BELL_WIDTH, ISLAND_HEIGHT),
                crate::UiAction::ToggleNotifications,
            );
            if let Some(image) = crate::status_icons::bell() {
                scene.push(crate::UiNode::Image {
                    id: crate::NodeId(40_000),
                    bounds: knave_ui::Rect::new(bell_x + 6.0, 10.0, 16.0, 16.0),
                    image,
                });
            }
        }
        if controls_visible {
            append_system_icons(&mut scene, system_x);
            scene.push(crate::UiNode::Panel {
                id: crate::NodeId(8_000),
                bounds: knave_ui::Rect::new(system_x, ISLAND_Y, SYSTEM_WIDTH, ISLAND_HEIGHT),
                color: Color::rgba(0, 0, 0, 0),
            });
        }
        if bell_visible {
            scene.push(crate::UiNode::Panel {
                id: crate::NodeId(4_001),
                bounds: knave_ui::Rect::new(bell_x, ISLAND_Y, BELL_WIDTH, ISLAND_HEIGHT),
                color: Color::rgba(0, 0, 0, 0),
            });
        }

        if bell_visible {
            append_bell(&mut scene, bell_x + BELL_WIDTH / 2.0, notifications);
        }
        let mut island_nodes = Vec::new();
        scene.nodes.retain(|node| {
            if is_island_node(node) {
                island_nodes.push(node.clone());
                false
            } else {
                true
            }
        });
        let inserted_islands = island_nodes.len();
        scene.nodes.splice(1..1, island_nodes);
        for index in &mut scene.brand_node_indices {
            if *index >= 1 {
                *index += inserted_islands;
            }
        }
        let theme = knave_ui::toolkit::Theme::Dark;
        let toolkit_root = build_toolkit_root(&scene, desktop);
        let toolkit_scene = build_toolkit_scene(toolkit_root.clone(), theme);
        let mut result = Self {
            scene,
            display: DisplayList::new(revision, Color::BACKGROUND),
            toolkit_scene,
            toolkit_root,
            size: [width, height],
            tinted_status_icons: None,
        };
        result.refresh_toolkit_output();
        result
    }
}

fn visible_workspaces(
    desktop: Option<&DesktopSnapshot>,
    start: f32,
    limit: f32,
) -> Vec<&knave_desktop_api::WorkspaceSummary> {
    let Some(snapshot) = desktop else {
        return Vec::new();
    };
    let capacity = (((limit - start - 8.0 + WORKSPACE_GAP) / (WORKSPACE_WIDTH + WORKSPACE_GAP))
        .floor()
        .max(0.0)) as usize;
    if capacity == 0 {
        return Vec::new();
    }
    let mut workspaces: Vec<_> = snapshot.workspaces.iter().collect();
    workspaces.sort_by_key(|workspace| workspace.workspace.0);
    let mut visible: Vec<_> = workspaces.into_iter().take(capacity).collect();
    if !visible.iter().any(|workspace| workspace.active)
        && let Some(active) = snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.active)
    {
        if visible.is_empty() {
            visible.push(active);
        } else {
            *visible.last_mut().unwrap() = active;
        }
        visible.sort_by_key(|workspace| workspace.workspace.0);
    }
    visible
}

fn focused_window(snapshot: &DesktopSnapshot) -> Option<&knave_desktop_api::WindowSummary> {
    let workspace = snapshot
        .workspaces
        .iter()
        .find(|workspace| workspace.active)?;
    snapshot.windows.iter().find(|window| {
        window.focused && !window.minimized && window.workspace == workspace.workspace
    })
}

fn append_system_icons(scene: &mut UiScene, x: f32) {
    if let Some(image) = crate::status_icons::wifi() {
        scene.push(crate::UiNode::Image {
            id: crate::NodeId(40_001),
            bounds: knave_ui::Rect::new(x + 5.0, 10.0, 16.0, 16.0),
            image,
        });
    }
    if let Some(image) = crate::status_icons::volume() {
        scene.push(crate::UiNode::Image {
            id: crate::NodeId(40_002),
            bounds: knave_ui::Rect::new(x + 27.0, 10.0, 16.0, 16.0),
            image,
        });
    }
}

fn element_at(id: u64, widget: Widget, bounds: Rect, class: &str, style_id: &str) -> Element {
    Element::new(id, widget)
        .class(class)
        .style_id(style_id)
        .layout(Layout {
            width: Length::Px(bounds.width),
            height: Length::Px(bounds.height),
            flow: Flow::Overlay,
            align: knave_ui::toolkit::Align::Start,
            offset: [bounds.x, bounds.y],
            ..Layout::default()
        })
}

fn action_class(action: crate::UiAction) -> &'static str {
    match action {
        crate::UiAction::OpenOverview => "statusbar__overview-action",
        crate::UiAction::FocusWorkspace(_) => "statusbar__workspace-action",
        crate::UiAction::ToggleNotifications => "statusbar__notifications-action",
        crate::UiAction::ToggleQuickSettings => "statusbar__quick-settings-action",
        crate::UiAction::Applet(_) => "statusbar__applet-action",
        _ => "statusbar__action",
    }
}

fn node_class(node: &crate::UiNode) -> &'static str {
    match node {
        crate::UiNode::Panel { id, .. } if id.0 == 4_000 => "statusbar__overview-island",
        crate::UiNode::Panel { id, .. } if id.0 == 4_001 => "statusbar__notification-island",
        crate::UiNode::Panel { id, .. } if id.0 == 4_999 => "statusbar__workspace-island",
        crate::UiNode::Panel { id, .. } if id.0 == 6_000 => "statusbar__applet-island",
        crate::UiNode::Panel { id, .. } if id.0 == 7_000 => "statusbar__focus-island",
        crate::UiNode::Panel { id, .. } if id.0 == 8_000 => "statusbar__system-island",
        crate::UiNode::Panel { id, .. } if id.0 == 40_010 => "statusbar__unread-badge",
        crate::UiNode::Panel { .. } => "statusbar__applet-attention",
        crate::UiNode::Label { id, .. } => match id.0 {
            8 => "statusbar__focused-title",
            40_011 => "statusbar__unread-count",
            40_012 => "statusbar__dnd-marker",
            40_013 => "statusbar__notification-state",
            10..=9_999 => "statusbar__workspace-label",
            20_000.. => "statusbar__applet-label",
            _ => "statusbar__content",
        },
        crate::UiNode::Image { id, .. } => match id.0 {
            3 => "statusbar__brand",
            9 => "statusbar__focused-icon",
            40_000 => "statusbar__notification-icon",
            40_001 => "statusbar__network-icon",
            40_002 => "statusbar__volume-icon",
            20_000.. => "statusbar__applet-icon",
            _ => "statusbar__content",
        },
    }
}

fn is_island_node(node: &crate::UiNode) -> bool {
    matches!(
        node,
        crate::UiNode::Panel { id, .. }
            if matches!(id.0, 4_000 | 4_001 | 4_999 | 6_000 | 7_000 | 8_000)
    )
}

fn build_toolkit_root(scene: &UiScene, desktop: Option<&DesktopSnapshot>) -> Element {
    let action_elements = scene.targets.iter().enumerate().map(|(index, target)| {
        let element_id = 50_000 + index as u64;
        let class = action_class(target.action);
        let (widget, variant) = if let crate::UiAction::FocusWorkspace(workspace_id) = target.action
        {
            let workspace = desktop.and_then(|snapshot| {
                snapshot
                    .workspaces
                    .iter()
                    .find(|workspace| workspace.workspace == workspace_id)
            });
            let variant = if workspace.is_some_and(|workspace| workspace.active) {
                ButtonVariant::Primary
            } else if workspace.is_some_and(|workspace| workspace.window_count > 0) {
                ButtonVariant::Secondary
            } else {
                ButtonVariant::Ghost
            };
            (Widget::Button(String::new()), Some(variant))
        } else {
            (Widget::Button(String::new()), None)
        };
        let mut element = element_at(element_id, widget, target.bounds, class, class);
        element.button_variant = variant;
        if let crate::UiAction::FocusWorkspace(workspace_id) = target.action {
            let label = if workspace_id.0 == 10 {
                "0".to_owned()
            } else {
                workspace_id.0.to_string()
            };
            element.semantics.name = Some(format!("Workspace {label}"));
        }
        element
    });
    let node_element = |node: &crate::UiNode| match node {
        crate::UiNode::Panel { id, .. } if id.0 == 1 => None,
        crate::UiNode::Panel { id, bounds, .. } => Some(element_at(
            60_000 + id.0,
            Widget::Panel,
            *bounds,
            node_class(node),
            node_class(node),
        )),
        crate::UiNode::Label {
            id, bounds, text, ..
        } => {
            let mut element = element_at(
                60_000 + id.0,
                Widget::Text(text.clone()),
                *bounds,
                node_class(node),
                node_class(node),
            )
            .text_wrap(knave_ui::TextWrap::None);
            if (10..=9_999).contains(&id.0)
                && let Some(workspace) = desktop.and_then(|snapshot| {
                    snapshot
                        .workspaces
                        .iter()
                        .find(|workspace| workspace.workspace.0 == (id.0 - 10) as u32)
                })
            {
                if workspace.active {
                    element = element.class("statusbar__workspace-label--active");
                }
                if workspace.window_count > 0 {
                    element = element.class("statusbar__workspace-label--occupied");
                }
            }
            Some(element)
        }
        crate::UiNode::Image { id, bounds, image } => Some(element_at(
            60_000 + id.0,
            Widget::Image(image.clone(), knave_ui::ImageStyle::default()),
            *bounds,
            node_class(node),
            node_class(node),
        )),
    };
    let island_elements = scene
        .nodes
        .iter()
        .filter(|node| is_island_node(node))
        .filter_map(node_element);
    let content_elements = scene
        .nodes
        .iter()
        .filter(|node| !is_island_node(node))
        .filter_map(node_element);
    let children = island_elements
        .chain(action_elements)
        .chain(content_elements)
        .collect();
    Element::new(1, Widget::Panel)
        .class("statusbar")
        .style_id("statusbar")
        .layout(Layout {
            width: Length::Fill,
            height: Length::Fill,
            flow: Flow::Overlay,
            align: knave_ui::toolkit::Align::Start,
            clip: knave_ui::toolkit::ClipMode::Visible,
            gap: 0.0,
            ..Layout::default()
        })
        .children(children)
}

fn build_toolkit_scene(root: Element, theme: knave_ui::toolkit::Theme) -> ToolkitScene {
    let mut toolkit_scene = ToolkitScene::new(crate::styles::root(root, theme))
        .expect("status bar element identities are unique");
    toolkit_scene
        .set_desktop_theme(theme, crate::styles::STYLESHEET)
        .expect("shared shell stylesheet is valid");
    toolkit_scene
}

struct StatusTextMetrics;

impl TextMeasurer for StatusTextMetrics {
    fn measure(&mut self, _: &str, style: &TextStyle, _: f32) -> TextLayout {
        // Status-bar text has fixed bounds and never participates in intrinsic sizing.
        TextLayout {
            size: [0.0, style.line_height],
            carets: Vec::new(),
        }
    }

    fn measure_spans(
        &mut self,
        spans: &[knave_ui::TextSpan],
        style: &TextStyle,
        width: f32,
    ) -> TextLayout {
        let text = spans
            .iter()
            .map(|span| span.text.as_str())
            .collect::<String>();
        self.measure(&text, style, width)
    }
}

/// Notification data remains in the shared scene; widget classes own its presentation.
fn append_bell(scene: &mut UiScene, midpoint: f32, snapshot: Option<&NotificationSnapshot>) {
    let x = midpoint - 14.0;
    let count = snapshot
        .filter(|s| s.owned)
        .map_or(0, NotificationSnapshot::unread);
    if count > 0 {
        let text = if count > 99 {
            "99+".to_owned()
        } else {
            count.to_string()
        };
        let w = if count > 99 { 16.0 } else { 14.0 };
        scene.push(crate::UiNode::Panel {
            id: crate::NodeId(40_010),
            bounds: Rect::new(x + 14.0, 4.0, w, 14.0),
            color: Color::ACCENT,
        });
        scene.push(crate::UiNode::Label {
            id: crate::NodeId(40_011),
            bounds: Rect::new(x + 14.0, 4.0, w, 14.0),
            color: Color::TEXT,
            text,
        });
    }
    if snapshot.is_some_and(|s| s.dnd) {
        scene.push(crate::UiNode::Label {
            id: crate::NodeId(40_012),
            bounds: Rect::new(x + 1.0, 22.0, 5.0, 10.0),
            color: Color::ACCENT,
            text: "z".into(),
        });
    } else if snapshot.is_none_or(|s| !s.owned) {
        scene.push(crate::UiNode::Label {
            id: crate::NodeId(40_013),
            bounds: Rect::new(x + 22.0, 22.0, 6.0, 10.0),
            color: Color::ACCENT,
            text: if snapshot.is_some() { "E" } else { "!" }.into(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{UiAction, UiNode};
    use knave_desktop_api::{WorkspaceId, WorkspaceSummary};

    fn desktop() -> DesktopSnapshot {
        DesktopSnapshot {
            generation: 1,
            overview_visible: false,
            workspaces: vec![
                WorkspaceSummary {
                    workspace: WorkspaceId(0),
                    active: false,
                    window_count: 0,
                    visible_window_count: 0,
                },
                WorkspaceSummary {
                    workspace: WorkspaceId(1),
                    active: true,
                    window_count: 2,
                    visible_window_count: 1,
                },
                WorkspaceSummary {
                    workspace: WorkspaceId(2),
                    active: false,
                    window_count: 1,
                    visible_window_count: 1,
                },
            ],
            windows: Vec::new(),
        }
    }

    fn applets(count: u64) -> Vec<Applet> {
        (1..=count)
            .map(|id| Applet {
                id,
                title: "Applet".into(),
                image: None,
                attention: false,
            })
            .collect()
    }

    #[test]
    fn compact_bar_orders_workspaces_applets_bell_and_system_controls() {
        let applets = applets(3);
        let bar = StatusBar::new(1, 1280.0, 36.0, Some(&desktop()), true, None, &applets);
        assert_eq!(bar.display.revision, bar.scene.revision());
        let workspace = bar
            .scene
            .hit_targets()
            .find_map(|(bounds, action)| {
                matches!(action, UiAction::FocusWorkspace(_)).then_some(bounds)
            })
            .unwrap();
        let tray = applet_targets(1280.0, &applets).next().unwrap().0;
        let applet_action = bar
            .scene
            .hit_targets()
            .find(|(_, action)| *action == UiAction::Applet(1))
            .expect("first applet remains actionable");
        let bell = bar
            .scene
            .hit_targets()
            .find_map(|(bounds, action)| {
                (action == UiAction::ToggleNotifications).then_some(bounds)
            })
            .unwrap();
        let system = bar
            .scene
            .hit_targets()
            .find_map(|(bounds, action)| {
                (action == UiAction::ToggleQuickSettings).then_some(bounds)
            })
            .unwrap();
        assert!(workspace.x < tray.x && tray.x < bell.x && bell.x < system.x);
        assert_eq!(system.x, 1224.0);
        assert_eq!(bell.x, 1190.0);
        assert_eq!(system.width, 48.0);
        assert_eq!(applet_action.0, tray);
        assert!(bar.scene.nodes().iter().any(|node| matches!(node,
            UiNode::Image { id, bounds, .. } if id.0 == 40_001 && bounds.width == 16.0)));
        assert!(bar.scene.nodes().iter().any(|node| matches!(node,
            UiNode::Image { id, bounds, .. } if id.0 == 40_002 && bounds.width == 16.0)));
        assert!(bar.scene.nodes().iter().any(|node| matches!(node,
            UiNode::Image { id, bounds, .. } if id.0 == 40_000 && bounds.width == 16.0)));
        assert!(!bar.scene.nodes().iter().any(|node| matches!(node,
            UiNode::Label { text, .. } if text == "Settings")));
    }

    #[test]
    fn theme_reload_updates_only_the_knave_mark_in_both_bar_render_paths() {
        let app_icon = knave_ui::UiImage::from_rgba(1, 1, vec![7, 11, 13, 255]).unwrap();
        let applets = [Applet {
            id: 1,
            title: "Applet".into(),
            image: Some(app_icon.clone()),
            attention: false,
        }];
        let mut bar = StatusBar::new(1, 800.0, 36.0, None, true, None, &applets);
        let light_mono = knave_ui::toolkit::ThemePolicy::new(
            knave_ui::toolkit::Theme::Light,
            knave_ui::toolkit::Contrast::High,
            knave_ui::toolkit::Chroma::Monochrome,
        );
        bar.set_user_policy(light_mono, "").unwrap();

        let scene_mark = bar
            .scene
            .nodes()
            .iter()
            .find_map(|node| match node {
                UiNode::Image { id, image, .. } if id.0 == 3 => Some(image),
                _ => None,
            })
            .expect("bar scene retains its registered brand mark");
        let expected_mark = crate::brand::image_for(light_mono).unwrap();
        assert_eq!(scene_mark.cache_key(), expected_mark.cache_key());
        assert!(bar.scene.display_list().commands.iter().any(|command| {
            matches!(command, knave_ui::DisplayCommand::Image { image, .. }
                if image.cache_key() == expected_mark.cache_key())
        }));
        assert!(bar.display.commands.iter().any(|command| {
            matches!(command, knave_ui::DisplayCommand::Image { image, .. }
                if image.cache_key() == expected_mark.cache_key())
        }));
        assert!(bar.display.commands.iter().any(|command| {
            matches!(command, knave_ui::DisplayCommand::Image { image, .. }
                if image.cache_key() == app_icon.cache_key())
        }));
    }

    #[test]
    fn scheme_setter_updates_brand_and_keeps_accessibility_and_user_css() {
        let mut bar = StatusBar::new(1, 800.0, 36.0, None, true, None, &[]);
        let css = ".statusbar { color: #123456; }";
        let initial = knave_ui::toolkit::ThemePolicy::new(
            knave_ui::toolkit::Theme::Dark,
            knave_ui::toolkit::Contrast::High,
            knave_ui::toolkit::Chroma::Monochrome,
        );
        bar.set_user_policy(initial, css).unwrap();
        bar.set_theme(knave_ui::toolkit::Theme::Light).unwrap();

        let actual = bar.scene.theme_policy();
        assert_eq!(actual.scheme, knave_ui::toolkit::Theme::Light);
        assert_eq!(actual.contrast, knave_ui::toolkit::Contrast::High);
        assert_eq!(actual.chroma, knave_ui::toolkit::Chroma::Monochrome);
        assert_eq!(bar.toolkit_scene.user_theme_stylesheet(), css);
        let expected = crate::brand::image_for(actual).unwrap();
        assert!(bar.display.commands.iter().any(|command| {
            matches!(command, knave_ui::DisplayCommand::Image { image, .. }
                if image.cache_key() == expected.cache_key())
        }));
    }

    #[test]
    fn statusbar_workspace_buttons_use_primary_secondary_and_ghost_variants() {
        let bar = StatusBar::new(1, 600.0, 36.0, Some(&desktop()), true, None, &[]);
        assert!(bar.scene.nodes().iter().any(|node| matches!(node,
            UiNode::Label { text, .. } if text == "0")));
        assert!(!bar.scene.nodes().iter().any(|node| matches!(node,
            UiNode::Panel { id, .. } if (5_000..6_000).contains(&id.0))));
        assert!(
            bar.display
                .commands
                .iter()
                .any(|command| matches!(command, knave_ui::DisplayCommand::Shape { .. }))
        );

        let variant_fill = |workspace: u32| {
            let index = bar
                .scene
                .targets
                .iter()
                .position(|target| {
                    target.action == UiAction::FocusWorkspace(WorkspaceId(workspace))
                })
                .expect("workspace action");
            bar.toolkit_scene
                .computed_style(ElementId(50_000 + index as u64))
                .unwrap()
                .style
                .background
                .and_then(|paint| paint.fill)
        };
        assert_eq!(variant_fill(1), Some(Color::ACCENT));
        assert!(variant_fill(2).is_some());
        assert_ne!(variant_fill(2), variant_fill(1));
        assert!(variant_fill(0).is_none_or(|color| color.alpha == 0));
        assert_eq!(
            bar.toolkit_scene
                .computed_style(ElementId(60_011))
                .unwrap()
                .style
                .text
                .color,
            Color::rgba(0x0A, 0x16, 0x20, 255),
            "the active workspace number should use dark on-accent text"
        );
    }

    #[test]
    fn high_contrast_monochrome_keeps_workspace_primary_legible_in_both_schemes() {
        let mut bar = StatusBar::new(1, 600.0, 36.0, Some(&desktop()), true, None, &[]);
        let workspace_style = |bar: &StatusBar, workspace: u32| {
            let index = bar
                .scene
                .targets
                .iter()
                .position(|target| {
                    target.action == UiAction::FocusWorkspace(WorkspaceId(workspace))
                })
                .expect("workspace action");
            bar.toolkit_scene
                .computed_style(ElementId(50_000 + index as u64))
                .unwrap()
                .style
        };

        let contrast_ratio = |first: Color, second: Color| {
            let luminance = |color: Color| {
                let channel = |value: u8| {
                    let value = f64::from(value) / 255.0;
                    if value <= 0.04045 {
                        value / 12.92
                    } else {
                        ((value + 0.055) / 1.055).powf(2.4)
                    }
                };
                0.2126 * channel(color.red)
                    + 0.7152 * channel(color.green)
                    + 0.0722 * channel(color.blue)
            };
            let first = luminance(first);
            let second = luminance(second);
            let (lighter, darker) = if first >= second {
                (first, second)
            } else {
                (second, first)
            };
            (lighter + 0.05) / (darker + 0.05)
        };

        for (scheme, expected_fill, expected_text, expected_inactive_text) in [
            (
                knave_ui::toolkit::Theme::Dark,
                Color::rgba(255, 255, 255, 255),
                Color::rgba(0, 0, 0, 255),
                Color::rgba(255, 255, 255, 255),
            ),
            (
                knave_ui::toolkit::Theme::Light,
                Color::rgba(0, 0, 0, 255),
                Color::rgba(255, 255, 255, 255),
                Color::rgba(0, 0, 0, 255),
            ),
        ] {
            bar.set_user_policy(
                knave_ui::toolkit::ThemePolicy::new(
                    scheme,
                    knave_ui::toolkit::Contrast::High,
                    knave_ui::toolkit::Chroma::Monochrome,
                ),
                "",
            )
            .unwrap();

            let active = workspace_style(&bar, 1);
            let inactive = workspace_style(&bar, 2);
            let active_fill = active
                .background
                .and_then(|paint| paint.fill)
                .expect("active workspace has a primary fill");
            let inactive_fill = inactive
                .background
                .and_then(|paint| paint.fill)
                .expect("occupied inactive workspace has a secondary fill");

            assert_eq!(active_fill, expected_fill);
            assert_eq!(active.text.color, expected_text);
            assert_eq!(inactive.text.color, expected_inactive_text);
            assert_ne!(active_fill, inactive_fill);
            assert_ne!(active.text.color, inactive.text.color);
            assert!(contrast_ratio(active.text.color, active_fill) >= 7.0);
        }
    }

    #[test]
    fn focused_title_keeps_its_widget_allocation_and_single_line_clip() {
        let mut desktop = desktop();
        desktop.windows.push(knave_desktop_api::WindowSummary {
            id: knave_desktop_api::WindowId(9),
            title: "文書作成 — a very long focused application title".into(),
            app_id: "org.example.Editor".into(),
            workspace: WorkspaceId(1),
            focused: true,
            minimized: false,
            floating: false,
            fullscreen: false,
            maximized: false,
        });
        let image = knave_ui::UiImage::from_rgba(1, 1, vec![1, 2, 3, 255]).unwrap();
        let bar = StatusBar::with_focused_icon(
            1,
            800.0,
            36.0,
            Some(&desktop),
            true,
            None,
            &[],
            Some(&image),
        );
        let (bounds, text) = bar
            .scene
            .nodes()
            .iter()
            .find_map(|node| match node {
                UiNode::Label {
                    id, bounds, text, ..
                } if id.0 == 8 => Some((*bounds, text)),
                _ => None,
            })
            .expect("focused title");
        assert!(bounds.width <= 205.0);
        assert!(bar.display.commands.iter().any(|command| matches!(command,
            knave_ui::DisplayCommand::Text { bounds: clip_bounds, style, clip: Some(clip), text: rendered, .. }
                if clip_bounds == &bounds && clip == &bounds && rendered == text
                    && style.wrap == knave_ui::TextWrap::None)));
        assert!(bar.scene.nodes().iter().any(|node| matches!(node,
            UiNode::Image { id, bounds, .. } if id.0 == 9 && bounds.width == 16.0)));
    }

    #[test]
    fn focused_section_yields_first_and_controls_do_not_overlap_at_narrow_widths() {
        let applets = applets(32);
        for width in [64.0, 80.0, 100.0, 140.0, 180.0, 250.0] {
            let bar = StatusBar::new(1, width, 36.0, Some(&desktop()), true, None, &applets);
            let mut targets: Vec<_> = bar.scene.hit_targets().map(|(bounds, _)| bounds).collect();
            targets.sort_by(|left, right| left.x.total_cmp(&right.x));
            assert!(
                targets
                    .iter()
                    .all(|bounds| bounds.x >= 0.0 && bounds.x + bounds.width <= width)
            );
            assert!(
                targets
                    .windows(2)
                    .all(|pair| pair[0].x + pair[0].width <= pair[1].x)
            );
            if width >= 200.0 {
                assert!(
                    bar.scene
                        .hit_targets()
                        .any(|(_, action)| { action == UiAction::FocusWorkspace(WorkspaceId(1)) })
                );
            }
            if width < 250.0 {
                assert!(!bar.scene.nodes().iter().any(|node| matches!(node,
                    UiNode::Label { id, .. } if id.0 == 8)));
            }
        }
    }

    #[test]
    fn disabled_notifications_release_the_bell_slot_for_applets() {
        let applets = applets(32);
        let with_bell: Vec<_> = applet_targets(800.0, &applets).collect();
        let without_bell: Vec<_> =
            applet_targets_with_notifications(800.0, &applets, false).collect();
        assert!(without_bell.len() >= with_bell.len());
        let bar = StatusBar::new(1, 800.0, 36.0, None, false, None, &applets);
        assert!(
            !bar.scene
                .hit_targets()
                .any(|(_, action)| action == UiAction::ToggleNotifications)
        );
        assert!(
            bar.scene
                .hit_targets()
                .any(|(_, action)| action == UiAction::ToggleQuickSettings)
        );
    }

    #[test]
    fn bundled_status_symbols_follow_theme_text_color_without_tinting_applets() {
        let applet_image =
            knave_ui::UiImage::from_rgba(48, 48, [7, 11, 13, 255].repeat(48 * 48)).unwrap();
        let applet = Applet {
            id: 7,
            title: "Third-party applet".into(),
            image: Some(applet_image.clone()),
            attention: false,
        };
        let mut bar = StatusBar::new(1, 800.0, 36.0, None, true, None, &[applet]);

        fn symbols<'a>(
            bar: &'a StatusBar,
            applet_image: &knave_ui::UiImage,
        ) -> Vec<&'a knave_ui::UiImage> {
            bar.display
                .commands
                .iter()
                .filter_map(|command| match command {
                    knave_ui::DisplayCommand::Image { image, .. }
                        if image.width() == 48 && image.height() == 48 && image != applet_image =>
                    {
                        Some(image)
                    }
                    _ => None,
                })
                .collect()
        }
        let check_color = |bar: &StatusBar, expected: [u8; 3]| {
            let symbols = symbols(bar, &applet_image);
            assert_eq!(symbols.len(), 3);
            for image in symbols {
                let visible_pixel = image
                    .pixels()
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .find(|pixel| pixel[3] > 200)
                    .expect("symbol contains an opaque pixel");
                assert_eq!(&visible_pixel[..3], &expected);
            }
            assert!(bar.display.commands.iter().any(|command| matches!(command,
                knave_ui::DisplayCommand::Image { image, .. } if image == &applet_image)));
        };

        check_color(&bar, [232, 235, 239]);
        bar.set_theme(knave_ui::toolkit::Theme::Light).unwrap();
        check_color(&bar, [23, 24, 28]);
        bar.set_theme(knave_ui::toolkit::Theme::Dark).unwrap();
        check_color(&bar, [232, 235, 239]);
        bar.set_stylesheet(
            ".statusbar__notification-icon { color: #123456; } \
             .statusbar__network-icon { color: #234567; } \
             .statusbar__volume-icon { color: #345678; }",
        )
        .unwrap();
        assert_eq!(
            bar.tinted_status_icons.as_ref().unwrap().0,
            [
                Color::rgba(0x12, 0x34, 0x56, 255),
                Color::rgba(0x23, 0x45, 0x67, 255),
                Color::rgba(0x34, 0x56, 0x78, 255),
            ],
            "each status glyph tint follows its computed CSS color"
        );

        let mut notifications_disabled = StatusBar::new(2, 800.0, 36.0, None, false, None, &[]);
        notifications_disabled
            .set_theme(knave_ui::toolkit::Theme::Light)
            .unwrap();
        let symbols = notifications_disabled
            .display
            .commands
            .iter()
            .filter_map(|command| match command {
                knave_ui::DisplayCommand::Image { image, .. }
                    if image.width() == 48 && image.height() == 48 =>
                {
                    Some(image)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(symbols.len(), 2);
        for image in symbols {
            let visible_pixel = image
                .pixels()
                .as_chunks::<4>()
                .0
                .iter()
                .find(|pixel| pixel[3] > 200)
                .expect("symbol contains an opaque pixel");
            assert_eq!(&visible_pixel[..3], &[23, 24, 28]);
        }
    }

    #[test]
    fn untitled_focus_keeps_its_icon_and_missing_focus_clears_the_island() {
        let mut snapshot = desktop();
        snapshot.windows.push(knave_desktop_api::WindowSummary {
            id: knave_desktop_api::WindowId(9),
            title: String::new(),
            app_id: String::new(),
            workspace: WorkspaceId(1),
            focused: true,
            minimized: false,
            floating: false,
            fullscreen: false,
            maximized: false,
        });
        let image = knave_ui::UiImage::from_rgba(1, 1, vec![1, 2, 3, 255]).unwrap();
        let compose = |snapshot: &DesktopSnapshot| {
            StatusBar::with_focused_icon(
                1,
                800.0,
                36.0,
                Some(snapshot),
                true,
                None,
                &[],
                Some(&image),
            )
        };
        let bar = compose(&snapshot);
        assert!(bar.scene.nodes().iter().any(|node| matches!(node,
            UiNode::Image { id, .. } if id.0 == 9)));
        assert!(!bar.scene.nodes().iter().any(|node| matches!(node,
            UiNode::Label { id, .. } if id.0 == 8)));
        snapshot.windows[0].focused = false;
        let bar = compose(&snapshot);
        assert!(!bar.scene.nodes().iter().any(|node| matches!(node,
            UiNode::Image { id, .. } if id.0 == 9)));
        assert!(!bar.scene.nodes().iter().any(|node| matches!(node,
            UiNode::Panel { id, .. } if id.0 == 7_000)));
    }

    #[test]
    fn statusbar_classes_resolve_widget_styles_and_interaction_states() {
        let mut snapshot = desktop();
        snapshot.windows.push(knave_desktop_api::WindowSummary {
            id: knave_desktop_api::WindowId(9),
            title: "CSS styled focus".into(),
            app_id: "org.example.Editor".into(),
            workspace: WorkspaceId(1),
            focused: true,
            minimized: false,
            floating: false,
            fullscreen: false,
            maximized: false,
        });
        let mut bar = StatusBar::new(1, 600.0, 36.0, Some(&snapshot), true, None, &[]);
        let active_index = bar
            .scene
            .targets
            .iter()
            .position(|target| target.action == UiAction::FocusWorkspace(WorkspaceId(1)))
            .unwrap();
        let idle_index = bar
            .scene
            .targets
            .iter()
            .position(|target| target.action == UiAction::FocusWorkspace(WorkspaceId(2)))
            .unwrap();
        let sheet = ".statusbar__workspace-action { margin: 1px; padding: 2px; border-radius: 3px; } \
            .statusbar__workspace-action.primary { background-color: #123456; } \
            .statusbar__workspace-action:hover { background-color: #abcdef; } \
            .statusbar__focused-title { font-size: 17px; }";
        bar.set_stylesheet(sheet).unwrap();
        let styled = bar.scene.targets[active_index].bounds;
        assert_eq!(
            bar.scene.hit_test(
                styled.x + styled.width / 2.0,
                styled.y + styled.height / 2.0
            ),
            Some(UiAction::FocusWorkspace(WorkspaceId(1)))
        );
        for (index, target) in bar.scene.targets.iter().enumerate() {
            assert_eq!(
                bar.toolkit_scene.bounds(ElementId(50_000 + index as u64)),
                Some(target.bounds),
                "action target follows its toolkit element"
            );
        }
        let active = bar
            .toolkit_scene
            .computed_style(ElementId(50_000 + active_index as u64))
            .unwrap();
        assert_eq!(active.padding, knave_ui::toolkit::Insets::all(2.0));
        assert_eq!(active.margin, knave_ui::toolkit::Insets::all(1.0));
        assert_eq!(active.style.radius, 3.0);
        assert_eq!(
            active
                .style
                .background
                .as_ref()
                .and_then(|paint| paint.fill),
            Some(Color::rgba(0x12, 0x34, 0x56, 255))
        );
        let hover = bar
            .toolkit_scene
            .computed_style_for(
                ElementId(50_000 + idle_index as u64),
                knave_ui::toolkit::InteractionState {
                    hovered: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(
            hover.style.background.as_ref().and_then(|paint| paint.fill),
            Some(Color::rgba(0xab, 0xcd, 0xef, 255))
        );
        assert_eq!(
            bar.toolkit_scene
                .computed_style(ElementId(60_008))
                .unwrap()
                .style
                .text
                .font_size,
            17.0
        );
        bar.set_theme(knave_ui::toolkit::Theme::Light).unwrap();
        assert_eq!(bar.toolkit_scene.theme(), knave_ui::toolkit::Theme::Light);
        let light_hover = bar
            .toolkit_scene
            .computed_style_for(
                ElementId(50_000 + idle_index as u64),
                knave_ui::toolkit::InteractionState {
                    hovered: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(
            light_hover
                .style
                .background
                .as_ref()
                .and_then(|paint| paint.fill),
            Some(Color::rgba(0xab, 0xcd, 0xef, 255)),
            "theme changes keep the application hover override"
        );
        assert_eq!(
            bar.toolkit_scene
                .computed_style(ElementId(50_000 + active_index as u64))
                .unwrap()
                .style
                .background
                .as_ref()
                .and_then(|paint| paint.fill),
            Some(Color::rgba(0x12, 0x34, 0x56, 255)),
            "theme changes keep the application stylesheet override"
        );
    }

    #[test]
    fn status_bar_uses_toolkit_palette_and_an_accent_notification_badge() {
        let mut snapshot = desktop();
        snapshot.workspaces[0].window_count = 1;
        snapshot.windows.push(knave_desktop_api::WindowSummary {
            id: knave_desktop_api::WindowId(9),
            title: "Focused editor".into(),
            app_id: "org.example.Editor".into(),
            workspace: WorkspaceId(1),
            focused: true,
            minimized: false,
            floating: false,
            fullscreen: false,
            maximized: false,
        });
        let mut notifications = NotificationSnapshot {
            owned: true,
            ..Default::default()
        };
        notifications
            .notifications
            .push(knave_desktop_api::notifications::Notification {
                unread: true,
                ..Default::default()
            });
        let applet = Applet {
            id: 7,
            title: "Fixture applet".into(),
            image: None,
            attention: false,
        };
        let mut bar = StatusBar::new(
            1,
            1280.0,
            36.0,
            Some(&snapshot),
            true,
            Some(&notifications),
            &[applet],
        );

        fn fill(bar: &StatusBar, id: u64) -> Option<Color> {
            bar.toolkit_scene
                .computed_style(ElementId(id))
                .unwrap_or_else(|| panic!("missing status bar toolkit element {id}"))
                .style
                .background
                .and_then(|paint| paint.fill)
        }
        fn palette(theme: knave_ui::toolkit::Theme, token: &str) -> Color {
            let mut scene = ToolkitScene::new(Element::new(1, Widget::Panel)).unwrap();
            scene.set_theme(theme).unwrap();
            scene
                .set_stylesheet(&format!("panel {{ background-color: var({token}); }}"))
                .unwrap();
            scene
                .computed_style(ElementId(1))
                .unwrap()
                .style
                .background
                .unwrap()
                .fill
                .unwrap()
        }
        for theme in [
            knave_ui::toolkit::Theme::Dark,
            knave_ui::toolkit::Theme::Light,
        ] {
            bar.set_theme(theme).unwrap();
            assert_eq!(fill(&bar, 1), Some(palette(theme, "--panel-surface")));
            assert_eq!(fill(&bar, 100_010), Some(palette(theme, "--accent")));
            let badge = bar
                .toolkit_scene
                .computed_style(ElementId(100_011))
                .unwrap();
            assert_eq!(badge.style.text.color, palette(theme, "--on-accent"));
            for id in [1, 64_000, 64_999, 66_000, 68_000, 64_001] {
                let style = bar.toolkit_scene.computed_style(ElementId(id)).unwrap();
                assert!(style.outline.is_none_or(|outline| outline.width == 0.0));
            }
        }
    }

    #[test]
    fn notification_count_and_dnd_state_remain_in_the_styled_scene() {
        let mut notifications = NotificationSnapshot {
            owned: true,
            dnd: true,
            ..Default::default()
        };
        notifications
            .notifications
            .push(knave_desktop_api::notifications::Notification {
                unread: true,
                ..Default::default()
            });
        let bar = StatusBar::new(
            1,
            800.0,
            36.0,
            Some(&desktop()),
            true,
            Some(&notifications),
            &[],
        );
        assert!(bar.scene.nodes().iter().any(|node| matches!(node,
            UiNode::Label { id, text, .. } if id.0 == 40_011 && text == "1")));
        assert!(bar.scene.nodes().iter().any(|node| matches!(node,
            UiNode::Label { id, text, .. } if id.0 == 40_012 && text == "z")));
        assert!(
            bar.scene
                .hit_targets()
                .any(|(_, action)| action == UiAction::ToggleNotifications)
        );
    }
}
