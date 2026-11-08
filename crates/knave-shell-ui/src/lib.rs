//! Desktop presentation owned by Knave Shell.
mod brand;
pub mod status_bar;
pub mod status_icons;
pub mod styles;
use knave_desktop_api::{DesktopSnapshot, WindowId, WindowSummary, WorkspaceId};
use knave_ui::{
    Color, DisplayList, DisplayListBuilder, ImageStyle, Rect, TextStyle, UiImage,
    toolkit::{Chroma, Contrast, Theme, ThemePolicy},
};
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspacePreviewImage {
    pub workspace: WorkspaceId,
    pub image: UiImage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiAction {
    OpenOverview,
    CloseOverview,
    ToggleNotifications,
    ToggleQuickSettings,
    Applet(u64),
    FocusWorkspace(WorkspaceId),
    FocusWindow(WindowId),
    RestoreWindow(WindowId),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct HitTarget {
    bounds: Rect,
    action: UiAction,
}

#[derive(Clone, Debug, PartialEq)]
pub enum UiNode {
    Panel {
        id: NodeId,
        bounds: Rect,
        color: Color,
    },
    Label {
        id: NodeId,
        bounds: Rect,
        color: Color,
        text: String,
    },
    Image {
        id: NodeId,
        bounds: Rect,
        image: UiImage,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct UiScene {
    revision: u64,
    clear_color: Color,
    theme_policy: ThemePolicy,
    nodes: Vec<UiNode>,
    brand_node_indices: Vec<usize>,
    targets: Vec<HitTarget>,
    search_actions: Vec<UiAction>,
}

const MAX_OVERVIEW_WINDOWS: usize = 32;
const MAX_OVERVIEW_WORKSPACES: usize = 10;
const MAX_SEARCH_RESULTS: usize = 12;
pub const MAX_SEARCH_QUERY: usize = 64;
const OVERVIEW_COLUMNS: usize = 4;
const OVERVIEW_STATUS_NODE_ID_OFFSET: u64 = 10_000;

fn window_label(window: &WindowSummary) -> String {
    let label = if window.title.is_empty() {
        window.app_id.as_str()
    } else {
        window.title.as_str()
    };
    bounded_text(label, 48)
}

fn bounded_text(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

#[derive(Clone, Debug)]
struct SearchResult {
    title: String,
    detail: String,
    action: UiAction,
}

fn search_results(snapshot: &DesktopSnapshot, query: &str) -> Vec<SearchResult> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }

    let mut results = Vec::with_capacity(MAX_SEARCH_RESULTS);
    for window in &snapshot.windows {
        let title = window_label(window);
        let app_id = if window.app_id.is_empty() {
            "Application".to_owned()
        } else {
            bounded_text(&window.app_id, 32)
        };
        let haystack =
            format!("{} {} workspace {}", title, app_id, window.workspace.0).to_lowercase();
        if haystack.contains(&needle) {
            let detail = if window.minimized {
                format!("{} · Restore minimized window", app_id)
            } else {
                format!("{} · Workspace {}", app_id, window.workspace.0)
            };
            results.push(SearchResult {
                title,
                detail: bounded_text(&detail, 64),
                action: if window.minimized {
                    UiAction::RestoreWindow(window.id)
                } else {
                    UiAction::FocusWindow(window.id)
                },
            });
            if results.len() == MAX_SEARCH_RESULTS {
                return results;
            }
        }
    }

    for workspace in &snapshot.workspaces {
        let title = format!("Workspace {}", workspace.workspace.0);
        if title.to_lowercase().contains(&needle) {
            results.push(SearchResult {
                title,
                detail: "Switch workspace".into(),
                action: UiAction::FocusWorkspace(workspace.workspace),
            });
            if results.len() == MAX_SEARCH_RESULTS {
                return results;
            }
        }
    }

    if "close overview".contains(&needle) || "escape".contains(&needle) {
        results.push(SearchResult {
            title: "Close overview".into(),
            detail: "Return to the desktop".into(),
            action: UiAction::CloseOverview,
        });
    }
    results
}

impl UiScene {
    pub fn new(revision: u64) -> Self {
        Self {
            revision,
            clear_color: Color::BACKGROUND,
            theme_policy: ThemePolicy::default(),
            nodes: Vec::new(),
            brand_node_indices: Vec::new(),
            targets: Vec::new(),
            search_actions: Vec::new(),
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn clear_color(&self) -> Color {
        self.clear_color
    }

    pub fn set_clear_color(&mut self, color: Color) {
        self.clear_color = color;
    }

    pub fn set_theme_policy(&mut self, policy: ThemePolicy) {
        self.theme_policy = policy;
        if !self.brand_node_indices.is_empty()
            && let Some(image) = brand::image_for(policy)
        {
            for index in &self.brand_node_indices {
                if let Some(UiNode::Image { image: mark, .. }) = self.nodes.get_mut(*index) {
                    *mark = image.clone();
                }
            }
        }
    }

    pub fn theme_policy(&self) -> ThemePolicy {
        self.theme_policy
    }

    pub fn nodes(&self) -> &[UiNode] {
        &self.nodes
    }

    pub fn push(&mut self, node: UiNode) {
        self.nodes.push(node);
    }

    /// Interaction regions in paint order, shared by retained and immediate hosts.
    pub fn hit_targets(&self) -> impl Iterator<Item = (Rect, UiAction)> + '_ {
        self.targets
            .iter()
            .map(|target| (target.bounds, target.action))
    }

    pub fn hit_test(&self, x: f32, y: f32) -> Option<UiAction> {
        self.targets
            .iter()
            .rev()
            .find(|target| {
                x >= target.bounds.x
                    && x < target.bounds.x + target.bounds.width
                    && y >= target.bounds.y
                    && y < target.bounds.y + target.bounds.height
            })
            .map(|target| target.action)
    }

    /// A disabled notification service must not leave an invisible clickable target.
    pub fn disable_notifications(&mut self) {
        self.targets
            .retain(|target| target.action != UiAction::ToggleNotifications);
    }
    pub fn search_action(&self, index: usize) -> Option<UiAction> {
        self.search_actions.get(index).copied()
    }

    pub fn search_result_count(&self) -> usize {
        self.search_actions.len()
    }

    fn target(&mut self, bounds: Rect, action: UiAction) {
        self.targets.push(HitTarget { bounds, action });
    }
    fn add_search_results(
        &mut self,
        snapshot: &DesktopSnapshot,
        query: &str,
        selected: usize,
        width: f32,
    ) {
        let results = search_results(snapshot, query);
        let panel_width = width.clamp(1.0, 690.0);
        let panel_x = ((width - panel_width) / 2.0).max(0.0);
        let row_height = 44.0;
        let panel_height = ((results.len().max(1) as f32) * row_height + 16.0).min(360.0);
        let panel = Rect::new(panel_x, 112.0, panel_width, panel_height);
        self.push(UiNode::Panel {
            id: NodeId(3_000),
            bounds: panel,
            color: Color::rgba(26, 36, 48, 245),
        });

        if results.is_empty() {
            let bounds = Rect::new(panel.x + 16.0, panel.y + 8.0, panel.width - 32.0, 28.0);
            self.push(UiNode::Label {
                id: NodeId(3_001),
                bounds,
                color: Color::TEXT,
                text: "No matching windows, workspaces, or actions".into(),
            });
            return;
        }

        let selected = selected.min(results.len() - 1);
        for (index, result) in results.into_iter().enumerate() {
            let bounds = Rect::new(
                panel.x + 8.0,
                panel.y + 8.0 + index as f32 * row_height,
                (panel.width - 16.0).max(1.0),
                40.0,
            );
            self.push(UiNode::Panel {
                id: NodeId(3_100 + index as u64),
                bounds,
                color: if index == selected {
                    Color::rgba(61, 90, 117, 255)
                } else {
                    Color::rgba(26, 36, 48, 0)
                },
            });
            let text = bounded_text(&format!("{} · {}", result.title, result.detail), 96);
            self.push(UiNode::Label {
                id: NodeId(3_200 + index as u64),
                bounds: Rect::new(
                    bounds.x + 10.0,
                    bounds.y + 4.0,
                    (bounds.width - 20.0).max(1.0),
                    32.0,
                ),
                color: Color::TEXT,
                text,
            });
            self.search_actions.push(result.action);
            self.target(bounds, result.action);
        }
    }

    pub fn bar(revision: u64, width: f32, height: f32) -> Self {
        let mut scene = Self::new(revision);
        scene.push(UiNode::Panel {
            id: NodeId(1),
            bounds: Rect::new(0.0, 0.0, width, height),
            color: Color::BACKGROUND,
        });
        // The icon-only Knave mark opens the Overview; no wordmark is painted.
        scene.target(Rect::new(0.0, 0.0, 40.0, height), UiAction::OpenOverview);
        if let Some(image) = brand::image() {
            scene.brand_node_indices.push(scene.nodes.len());
            scene.push(UiNode::Image {
                id: NodeId(3),
                bounds: Rect::new(11.0, 7.0, 22.0, 22.0),
                image,
            });
        }
        scene
    }

    pub fn bar_with_snapshot(
        revision: u64,
        width: f32,
        height: f32,
        snapshot: Option<&DesktopSnapshot>,
    ) -> Self {
        status_bar::StatusBar::new(revision, width, height, snapshot, true, None, &[]).scene
    }

    pub fn overview(revision: u64, width: f32, height: f32) -> Self {
        let mut scene = Self::overview_background(revision, width, height);
        scene.append_status_bar(width, None);
        scene
    }

    fn overview_background(revision: u64, width: f32, height: f32) -> Self {
        let mut scene = Self::new(revision);
        scene.set_clear_color(Color::rgba(8, 12, 18, 245));
        scene.push(UiNode::Panel {
            id: NodeId(1),
            bounds: Rect::new(0.0, 0.0, width, height),
            color: Color::BACKGROUND,
        });
        scene.target(Rect::new(0.0, 0.0, width, height), UiAction::CloseOverview);
        scene
    }

    fn append_status_bar(&mut self, width: f32, snapshot: Option<&DesktopSnapshot>) {
        let mut status_bar = Self::bar_with_snapshot(
            self.revision,
            width,
            status_bar::STATUS_BAR_HEIGHT,
            snapshot,
        );
        for node in &mut status_bar.nodes {
            match node {
                UiNode::Panel { id, .. } | UiNode::Label { id, .. } | UiNode::Image { id, .. } => {
                    id.0 += OVERVIEW_STATUS_NODE_ID_OFFSET
                }
            }
        }
        let node_offset = self.nodes.len();
        self.brand_node_indices.extend(
            status_bar
                .brand_node_indices
                .iter()
                .map(|index| node_offset + index),
        );
        self.nodes.append(&mut status_bar.nodes);
        self.targets.append(&mut status_bar.targets);
    }
    pub fn overview_with_snapshot(
        revision: u64,
        width: f32,
        height: f32,
        snapshot: Option<&DesktopSnapshot>,
    ) -> Self {
        Self::overview_with_snapshot_and_search(revision, width, height, snapshot, "", 0, &[])
    }

    pub fn overview_with_snapshot_and_search(
        revision: u64,
        width: f32,
        height: f32,
        snapshot: Option<&DesktopSnapshot>,
        query: &str,
        selected: usize,
        previews: &[WorkspacePreviewImage],
    ) -> Self {
        let mut scene = Self::overview_background(revision, width, height);
        scene.append_status_bar(width, snapshot);
        if let Some(snapshot) = snapshot
            && let Some(workspace) = snapshot
                .workspaces
                .iter()
                .find(|workspace| workspace.active)
        {
            scene.push(UiNode::Label {
                id: NodeId(100),
                bounds: Rect::new(32.0, 64.0, width - 64.0, height - 96.0),
                color: Color::TEXT,
                text: if query.is_empty() {
                    format!("Overview · {} windows", workspace.window_count)
                } else {
                    format!("Search: {}", bounded_text(query, MAX_SEARCH_QUERY))
                },
            });

            let gap = 16.0;
            let workspace_count = snapshot.workspaces.len().min(MAX_OVERVIEW_WORKSPACES);
            let mut window_top = 112.0;
            if workspace_count > 0 {
                let preview_width = ((width - 64.0 - gap * (workspace_count as f32 - 1.0))
                    / workspace_count as f32)
                    .max(1.0);
                let preview_height = (preview_width * 9.0 / 16.0).clamp(90.0, 260.0);
                for (index, workspace) in
                    snapshot.workspaces.iter().take(workspace_count).enumerate()
                {
                    let bounds = Rect::new(
                        32.0 + index as f32 * (preview_width + gap),
                        112.0,
                        preview_width,
                        preview_height,
                    );
                    scene.push(UiNode::Panel {
                        id: NodeId(500 + index as u64),
                        bounds,
                        color: if workspace.active {
                            Color::ACCENT
                        } else {
                            Color::rgba(35, 48, 64, 255)
                        },
                    });
                    if let Some(preview) = previews
                        .iter()
                        .find(|preview| preview.workspace == workspace.workspace)
                    {
                        scene.push(UiNode::Image {
                            id: NodeId(600 + index as u64),
                            bounds: Rect::new(
                                bounds.x + 2.0,
                                bounds.y + 2.0,
                                (bounds.width - 4.0).max(1.0),
                                (bounds.height - 28.0).max(1.0),
                            ),
                            image: preview.image.clone(),
                        });
                    }
                    scene.push(UiNode::Label {
                        id: NodeId(700 + index as u64),
                        bounds: Rect::new(
                            bounds.x + 8.0,
                            bounds.y + bounds.height - 24.0,
                            (bounds.width - 16.0).max(1.0),
                            20.0,
                        ),
                        color: Color::TEXT,
                        text: format!("Workspace {}", workspace.workspace.0),
                    });
                    scene.target(bounds, UiAction::FocusWorkspace(workspace.workspace));
                }
                window_top = 112.0 + preview_height + 40.0;
            }

            let card_width = ((width - 64.0 - gap * (OVERVIEW_COLUMNS as f32 - 1.0))
                / OVERVIEW_COLUMNS as f32)
                .max(1.0);
            let card_height = 72.0;
            for (index, window) in snapshot
                .windows
                .iter()
                .filter(|window| window.workspace == workspace.workspace)
                .take(MAX_OVERVIEW_WINDOWS)
                .enumerate()
            {
                let column = index % OVERVIEW_COLUMNS;
                let row = index / OVERVIEW_COLUMNS;
                let bounds = Rect::new(
                    32.0 + column as f32 * (card_width + gap),
                    window_top + row as f32 * (card_height + gap),
                    card_width,
                    card_height,
                );
                scene.push(UiNode::Panel {
                    id: NodeId(1_000 + index as u64),
                    bounds,
                    color: if window.focused {
                        Color::ACCENT
                    } else {
                        Color::rgba(35, 48, 64, 255)
                    },
                });
                scene.push(UiNode::Label {
                    id: NodeId(2_000 + index as u64),
                    bounds: Rect::new(
                        bounds.x + 12.0,
                        bounds.y + 8.0,
                        (bounds.width - 24.0).max(1.0),
                        (bounds.height - 16.0).max(1.0),
                    ),
                    color: Color::TEXT,
                    text: window_label(window),
                });
                scene.target(
                    bounds,
                    if window.minimized {
                        UiAction::RestoreWindow(window.id)
                    } else {
                        UiAction::FocusWindow(window.id)
                    },
                );
            }
            if !query.is_empty() {
                scene.add_search_results(snapshot, query, selected, width);
            }
        }
        scene
    }
}

impl Default for UiScene {
    fn default() -> Self {
        Self::new(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_scene_is_renderer_independent() {
        let scene = UiScene::bar(7, 1920.0, 36.0);
        assert_eq!(scene.revision(), 7);
        assert_eq!(scene.nodes().len(), 2);
        // The brand mark is always resolvable from the bundled asset.
        assert!(
            scene
                .nodes()
                .iter()
                .any(|node| matches!(node, UiNode::Image { .. }))
        );
    }

    #[test]
    fn overview_theme_policy_changes_only_its_registered_brand_node() {
        let app_image = UiImage::from_rgba(1, 1, vec![12, 34, 56, 255]).unwrap();
        let mut scene = UiScene::overview(1, 800.0, 600.0);
        scene.push(UiNode::Image {
            id: NodeId(90),
            bounds: Rect::new(1.0, 1.0, 1.0, 1.0),
            image: app_image.clone(),
        });
        let policy = ThemePolicy::new(Theme::Light, Contrast::High, Chroma::Monochrome);
        scene.set_theme_policy(policy);

        let expected = brand::image_for(policy).unwrap();
        assert!(scene.nodes().iter().any(|node| matches!(node,
            UiNode::Image { id: NodeId(10_003), image, .. }
                if image.cache_key() == expected.cache_key()
        )));
        assert!(scene.nodes().iter().any(|node| matches!(node,
            UiNode::Image { id: NodeId(90), image, .. }
                if image.cache_key() == app_image.cache_key()
        )));
    }

    #[test]
    fn unknown_workspace_state_never_adds_a_wordmark() {
        let snapshot = DesktopSnapshot {
            generation: 0,
            overview_visible: false,
            workspaces: vec![],
            windows: vec![],
        };
        for scene in [
            UiScene::bar(0, 800.0, 36.0),
            UiScene::bar_with_snapshot(0, 800.0, 36.0, None),
            UiScene::bar_with_snapshot(0, 800.0, 36.0, Some(&snapshot)),
        ] {
            assert!(!scene.nodes().iter().any(|node| matches!(node,
                UiNode::Label { text, .. } if text.starts_with("Workspace "))));
        }
    }

    #[test]
    fn overview_scene_does_not_require_a_gpu() {
        let scene = UiScene::overview(1, 1920.0, 1080.0);
        assert_eq!(scene.clear_color(), Color::rgba(8, 12, 18, 245));
        assert_eq!(scene.nodes().len(), 10);
        assert_eq!(
            scene.nodes()[0],
            UiNode::Panel {
                id: NodeId(1),
                bounds: Rect::new(0.0, 0.0, 1920.0, 1080.0),
                color: Color::BACKGROUND,
            }
        );
        assert!(matches!(
            &scene.nodes()[1],
            UiNode::Panel { bounds, .. }
                if *bounds == Rect::new(0.0, 0.0, 1920.0, 36.0)
        ));
        assert!(scene.nodes().iter().any(|node| matches!(node,
            UiNode::Image { bounds, .. }
                if *bounds == Rect::new(11.0, 7.0, 22.0, 22.0)
        )));
        // The icon-only mark introduces the overview from the shared strip.
        assert_eq!(scene.hit_test(20.0, 18.0), Some(UiAction::OpenOverview));
    }
    #[test]
    fn snapshot_scene_reflects_active_workspace() {
        let snapshot = DesktopSnapshot {
            generation: 4,
            overview_visible: true,
            workspaces: vec![knave_desktop_api::WorkspaceSummary {
                workspace: knave_desktop_api::WorkspaceId(2),
                active: true,
                window_count: 3,
                visible_window_count: 2,
            }],
            windows: vec![
                WindowSummary {
                    id: WindowId(41),
                    title: "Terminal".into(),
                    app_id: "foot".into(),
                    workspace: WorkspaceId(2),
                    focused: true,
                    minimized: false,
                    floating: false,
                    fullscreen: false,
                    maximized: false,
                },
                WindowSummary {
                    id: WindowId(42),
                    title: "Editor".into(),
                    app_id: "code".into(),
                    workspace: WorkspaceId(2),
                    focused: false,
                    minimized: true,
                    floating: false,
                    fullscreen: false,
                    maximized: false,
                },
            ],
        };
        let bar = UiScene::bar_with_snapshot(4, 1920.0, 36.0, Some(&snapshot));
        assert!(bar.nodes().iter().any(|node| matches!(node,
            UiNode::Label { bounds, text, .. }
                if *bounds == Rect::new(46.0, 7.0, 22.0, 22.0) && text == "2")));
        let overview = UiScene::overview_with_snapshot(4, 1920.0, 1080.0, Some(&snapshot));
        assert!(overview.nodes().iter().any(|node| matches!(node,
            UiNode::Label { bounds, text, .. }
                if *bounds == Rect::new(46.0, 7.0, 22.0, 22.0) && text == "2")));
        assert_eq!(overview.hit_test(20.0, 18.0), Some(UiAction::OpenOverview));
        assert_eq!(
            overview.hit_test(40.0, 420.0),
            Some(UiAction::FocusWindow(WindowId(41)))
        );
        assert_eq!(
            overview.hit_test(500.0, 420.0),
            Some(UiAction::RestoreWindow(WindowId(42)))
        );
        assert_eq!(
            overview.hit_test(57.0, 15.0),
            Some(UiAction::FocusWorkspace(WorkspaceId(2)))
        );
        assert_eq!(overview.hit_test(40.0, 25.0), Some(UiAction::CloseOverview));
        assert_eq!(
            overview.hit_test(1000.0, 700.0),
            Some(UiAction::CloseOverview)
        );
    }

    #[test]
    fn overview_scene_includes_bounded_workspace_preview() {
        let snapshot = DesktopSnapshot {
            generation: 1,
            overview_visible: true,
            workspaces: vec![knave_desktop_api::WorkspaceSummary {
                workspace: WorkspaceId(1),
                active: true,
                window_count: 0,
                visible_window_count: 0,
            }],
            windows: Vec::new(),
        };
        let preview = WorkspacePreviewImage {
            workspace: WorkspaceId(1),
            image: UiImage::from_rgba(320, 180, vec![0; 320 * 180 * 4]).unwrap(),
        };
        let scene = UiScene::overview_with_snapshot_and_search(
            1,
            1920.0,
            1080.0,
            Some(&snapshot),
            "",
            0,
            &[preview],
        );

        assert!(
            scene
                .nodes()
                .iter()
                .any(|node| matches!(node, UiNode::Image {
            image,
            ..
        } if image.width() == 320 && image.height() == 180))
        );
    }

    #[test]
    fn search_results_provide_bounded_actions() {
        let snapshot = DesktopSnapshot {
            generation: 1,
            overview_visible: true,
            workspaces: vec![knave_desktop_api::WorkspaceSummary {
                workspace: WorkspaceId(1),
                active: true,
                window_count: 1,
                visible_window_count: 1,
            }],
            windows: vec![WindowSummary {
                id: WindowId(9),
                title: "Editor".into(),
                app_id: "code".into(),
                workspace: WorkspaceId(1),
                focused: false,
                minimized: true,
                floating: false,
                fullscreen: false,
                maximized: false,
            }],
        };
        let scene = UiScene::overview_with_snapshot_and_search(
            1,
            1920.0,
            1080.0,
            Some(&snapshot),
            "editor",
            0,
            &[],
        );
        assert_eq!(scene.search_result_count(), 1);
        assert_eq!(
            scene.search_action(0),
            Some(UiAction::RestoreWindow(WindowId(9)))
        );
        assert_eq!(
            scene.hit_test(960.0, 130.0),
            Some(UiAction::RestoreWindow(WindowId(9)))
        );
    }
}

impl UiScene {
    pub fn display_list(&self) -> DisplayList {
        let scene = self;
        let mut builder = DisplayListBuilder::new(
            scene.revision(),
            appearance_color(scene.clear_color(), scene.theme_policy, ColorRole::Surface),
        );
        for node in scene.nodes() {
            match node {
                UiNode::Panel { bounds, color, .. } => {
                    builder.fill_rect(
                        *bounds,
                        appearance_color(*color, scene.theme_policy, ColorRole::Surface),
                    );
                }
                UiNode::Label {
                    bounds,
                    color,
                    text,
                    ..
                } => {
                    builder.text(
                        *bounds,
                        text.clone(),
                        TextStyle {
                            color: appearance_color(*color, scene.theme_policy, ColorRole::Text),
                            ..TextStyle::default()
                        },
                    );
                }
                UiNode::Image { bounds, image, .. } => {
                    builder.image(*bounds, image.clone(), ImageStyle::default());
                }
            }
        }
        builder
            .finish()
            .expect("scene conversion does not alter builder state")
    }
}

#[derive(Clone, Copy)]
enum ColorRole {
    Surface,
    Text,
}

fn appearance_color(color: Color, policy: ThemePolicy, role: ColorRole) -> Color {
    if policy.contrast == Contrast::High {
        if matches!(role, ColorRole::Text) {
            return match policy.scheme {
                Theme::Dark => Color::rgba(255, 255, 255, color.alpha),
                Theme::Light => Color::rgba(0, 0, 0, color.alpha),
            };
        }
        if color.alpha == 0 {
            return color;
        }
        if color == Color::ACCENT {
            return match (policy.scheme, policy.chroma) {
                (Theme::Dark, Chroma::FullColor) => Color::rgba(0, 255, 255, color.alpha),
                (Theme::Light, Chroma::FullColor) => Color::rgba(0, 95, 204, color.alpha),
                (_, Chroma::Monochrome) => Color::rgba(128, 128, 128, color.alpha),
            };
        }
        return match policy.scheme {
            Theme::Dark => Color::rgba(0, 0, 0, color.alpha),
            Theme::Light => Color::rgba(255, 255, 255, color.alpha),
        };
    }

    if policy.chroma == Chroma::Monochrome {
        let gray = ((u32::from(color.red) * 2126
            + u32::from(color.green) * 7152
            + u32::from(color.blue) * 722)
            / 10_000) as u8;
        Color::rgba(gray, gray, gray, color.alpha)
    } else {
        color
    }
}

#[cfg(test)]
mod projection_tests {
    use super::*;
    use knave_ui::DisplayCommand;

    #[test]
    fn monochrome_policy_grays_shell_chrome_and_preserves_image_content() {
        let mut scene = UiScene::new(1);
        scene.set_clear_color(Color::rgba(8, 12, 18, 255));
        scene.push(UiNode::Panel {
            id: NodeId(1),
            bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
            color: Color::rgba(26, 36, 48, 255),
        });
        scene.push(UiNode::Label {
            id: NodeId(2),
            bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
            color: Color::rgba(93, 173, 226, 255),
            text: "Shell label".into(),
        });
        let image = UiImage::from_rgba(1, 1, vec![93, 173, 226, 255]).unwrap();
        scene.push(UiNode::Image {
            id: NodeId(3),
            bounds: Rect::new(0.0, 0.0, 1.0, 1.0),
            image: image.clone(),
        });
        scene.set_theme_policy(ThemePolicy::new(
            Theme::Dark,
            Contrast::Standard,
            Chroma::Monochrome,
        ));
        let list = scene.display_list();
        let grayscale = |color: Color| color.red == color.green && color.green == color.blue;
        assert!(grayscale(list.clear_color));
        assert!(list.commands.iter().all(|command| {
            match command {
                DisplayCommand::Shape { paint, .. } => paint.fill.is_none_or(grayscale),
                DisplayCommand::Text { style, .. } => grayscale(style.color),
                DisplayCommand::Image { image: painted, .. } => painted == &image,
                DisplayCommand::RichText { spans, .. } => {
                    spans.iter().all(|span| span.color.is_none_or(grayscale))
                }
                DisplayCommand::BeginOpacity { .. } | DisplayCommand::EndOpacity => true,
            }
        }));
    }

    #[test]
    fn high_contrast_policy_uses_extreme_shell_text_and_surface_colors() {
        let mut scene = UiScene::new(1);
        scene.push(UiNode::Panel {
            id: NodeId(1),
            bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
            color: Color::ACCENT,
        });
        scene.push(UiNode::Label {
            id: NodeId(2),
            bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
            color: Color::rgba(1, 2, 3, 255),
            text: "Shell label".into(),
        });
        scene.set_theme_policy(ThemePolicy::new(
            Theme::Dark,
            Contrast::High,
            Chroma::FullColor,
        ));
        let list = scene.display_list();
        assert!(matches!(
            list.commands.first(),
            Some(DisplayCommand::Shape { paint, .. })
                if paint.fill == Some(Color::rgba(0, 255, 255, 255))
        ));
        assert!(matches!(
            list.commands.get(1),
            Some(DisplayCommand::Text { style, .. })
                if style.color == Color::rgba(255, 255, 255, 255)
        ));
    }
    #[test]
    fn scene_projection_preserves_order_revision_and_clear_color() {
        let mut scene = UiScene::new(11);
        scene.set_clear_color(Color::rgba(8, 12, 18, 245));
        scene.push(UiNode::Panel {
            id: NodeId(1),
            bounds: Rect::new(0.0, 0.0, 4.0, 4.0),
            color: Color::ACCENT,
        });
        scene.push(UiNode::Label {
            id: NodeId(2),
            bounds: Rect::new(1.0, 1.0, 2.0, 2.0),
            color: Color::TEXT,
            text: "top".into(),
        });
        let list = scene.display_list();
        assert_eq!(list.revision, 11);
        assert_eq!(list.clear_color, Color::rgba(8, 12, 18, 245));
        assert!(matches!(list.commands[0], DisplayCommand::Shape { .. }));
        assert!(matches!(list.commands[1], DisplayCommand::Text { .. }));
    }
}
