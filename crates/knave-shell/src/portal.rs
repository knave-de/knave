//! Native portal selection and sharing controls on Knave's retained UI toolkit.
use base64::{Engine, engine::general_purpose::STANDARD};
use knave_portal_api::{MAX_REQUEST, Operation, Reply, Request, SourceKind};
use knave_ui::{DisplayList, TextWrap, UiImage, toolkit::*};
use knave_wayland::{Application, KeyboardMode, SurfaceLayer, SurfaceOptions, SurfacePosition};
use std::{
    collections::BTreeSet,
    io::{self, Read},
};
const CANCEL: ElementId = ElementId(2);
const CONFIRM: ElementId = ElementId(3);
const SOURCE: u64 = 100;
const TAB: u64 = 20;
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Screen,
    Window,
    Workspaces,
}
impl Mode {
    fn matches(self, kind: SourceKind) -> bool {
        matches!(
            (self, kind),
            (Self::Screen, SourceKind::Monitor)
                | (Self::Window, SourceKind::Window)
                | (Self::Workspaces, SourceKind::Workspace { .. })
        )
    }
}
pub struct Picker {
    theme: Theme,
    workspace_frozen: bool,
    mode: Mode,
    request: Request,
    selected: BTreeSet<u32>,
    images: Vec<Option<UiImage>>,
    scene: Scene,
    dirty: bool,
    closed: bool,
    size: [f32; 2],
}
impl Picker {
    fn direct_share(&self) -> bool {
        self.request.operation == Operation::Share
            && !self.request.multiple
            && self.mode != Mode::Workspaces
    }

    pub fn new(request: Request) -> Result<Self, String> {
        Self::with_theme(request, Theme::Dark)
    }

    pub fn with_theme(request: Request, theme: Theme) -> Result<Self, String> {
        request.validate()?;
        let images = request
            .sources
            .iter()
            .map(|source| {
                let encoded = source.preview_png.as_ref()?;
                let data = STANDARD.decode(encoded).ok()?;
                let mut decoder = png::Decoder::new(std::io::Cursor::new(data));
                decoder.set_limits(png::Limits { bytes: 1024 * 1024 });
                let mut reader = decoder.read_info().ok()?;
                if reader.info().width > 320 || reader.info().height > 180 {
                    return None;
                }
                let mut pixels = vec![0; reader.output_buffer_size()?];
                let info = reader.next_frame(&mut pixels).ok()?;
                if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight
                {
                    return None;
                }
                pixels.truncate(info.buffer_size());
                UiImage::from_rgba(info.width, info.height, pixels)
            })
            .collect();
        let mut scene = Scene::new(Element::new(0, Widget::Panel)).map_err(|e| e.to_string())?;
        scene
            .set_desktop_theme(theme, knave_shell_ui::styles::STYLESHEET)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            theme,
            workspace_frozen: false,
            mode: if request
                .sources
                .iter()
                .any(|source| source.kind == SourceKind::Monitor)
            {
                Mode::Screen
            } else if request
                .sources
                .iter()
                .any(|source| source.kind == SourceKind::Window)
            {
                Mode::Window
            } else {
                Mode::Workspaces
            },
            request,
            images,
            selected: BTreeSet::new(),
            scene,
            dirty: true,
            closed: false,
            size: [0.0; 2],
        })
    }
    fn respond(&mut self, selected: Vec<u32>) {
        if self.closed {
            return;
        }
        let reply = Reply {
            request_id: self.request.request_id.clone(),
            selected,
        };
        if let Ok(json) = serde_json::to_string(&reply) {
            println!("{json}");
        }
        self.closed = true;
    }
    fn compose(&self) -> Element {
        if self.request.operation == Operation::Sharing {
            return self.sharing_control();
        }
        let mut root = Element::new(0, Widget::Panel).layout(Layout {
            padding: 24.0,
            gap: 16.0,
            ..Default::default()
        });
        let title = match self.request.operation {
            Operation::Share => "Choose what to share",
            Operation::Screenshot => "Choose a screen to capture",
            Operation::Sharing => "Screen sharing is active",
        };
        root.children
            .push(Element::new(4, Widget::Text(title.into())).layout(Layout {
                height: Length::Px(32.0),
                ..Default::default()
            }));
        let app = if self.request.app_id.is_empty() {
            "An application"
        } else {
            &self.request.app_id
        };
        root.children.push(
            Element::new(
                5,
                Widget::Text(format!(
                    "{app} — {}",
                    if self.request.operation == Operation::Sharing {
                        "can see the selected screens"
                    } else {
                        "will see only what you approve"
                    }
                )),
            )
            .layout(Layout {
                height: Length::Px(48.0),
                ..Default::default()
            }),
        );
        if self.request.operation == Operation::Share {
            let mut tabs = Element::new(10, Widget::Panel)
                .without_surface()
                .class("shell-structure")
                .layout(Layout {
                    flow: Flow::Row,
                    height: Length::Px(38.0),
                    gap: 8.0,
                    ..Default::default()
                });
            for (index, (mode, label)) in [
                (Mode::Screen, "Screen"),
                (Mode::Window, "Window"),
                (Mode::Workspaces, "Workspaces"),
            ]
            .into_iter()
            .enumerate()
            {
                let mut button = Element::new(TAB + index as u64, Widget::Button(label.into()));
                button.enabled = self
                    .request
                    .sources
                    .iter()
                    .any(|source| mode.matches(source.kind));
                if self.mode == mode {
                    button.classes.push("portal-selected".into());
                }
                tabs.children.push(button);
            }
            root.children.push(tabs);
            if self.mode == Mode::Workspaces {
                root.children.push(Element::new(11, Widget::Text("Share selected workspaces. Switching elsewhere freezes the last shared frame.".into())).layout(Layout { height: Length::Px(44.0), ..Default::default() }));
            }
        }
        if self.request.operation != Operation::Sharing {
            let mut grid = Element::new(6, Widget::Panel)
                .without_surface()
                .class("shell-structure")
                .layout(Layout {
                    flow: Flow::Grid {
                        columns: if self.mode == Mode::Workspaces { 5 } else { 2 },
                    },
                    gap: 12.0,
                    height: Length::Auto,
                    ..Default::default()
                });
            for (index, source) in self.request.sources.iter().enumerate() {
                if !self.mode.matches(source.kind) {
                    continue;
                }
                let selected = self.selected.contains(&source.id);
                if let SourceKind::Workspace { workspace } = source.kind {
                    let mut checkbox = Element::new(
                        SOURCE + index as u64,
                        Widget::Button(format!(
                            "{}Workspace {workspace}",
                            if selected { "✓ " } else { "" }
                        )),
                    )
                    .layout(Layout {
                        height: Length::Px(72.0),
                        padding: 8.0,
                        ..Default::default()
                    });
                    if selected {
                        checkbox.classes.push("portal-selected".into());
                    }
                    grid.children.push(checkbox);
                    continue;
                }
                let mut card = Element::new(SOURCE + index as u64, Widget::Button(String::new()))
                    .class("portal-card")
                    .layout(Layout {
                        height: Length::Px(190.0),
                        padding: 10.0,
                        gap: 8.0,
                        ..Default::default()
                    });
                if selected {
                    card.classes.push("portal-selected".into());
                }
                if let Some(image) = &self.images[index] {
                    card.children.push(
                        Element::new(
                            1000 + index as u64,
                            Widget::Image(image.clone(), Default::default()),
                        )
                        .layout(Layout {
                            height: Length::Px(120.0),
                            ..Default::default()
                        }),
                    );
                }
                card.children.push(
                    Element::new(
                        2000 + index as u64,
                        Widget::Text(format!(
                            "{}{}\n{} × {}",
                            if selected { "✓ " } else { "" },
                            source.name,
                            source.width,
                            source.height
                        )),
                    )
                    .layout(Layout {
                        height: Length::Px(44.0),
                        ..Default::default()
                    }),
                );
                if self.direct_share() {
                    card.children.push(
                        Element::new(
                            3000 + index as u64,
                            Widget::Text(
                                if source.kind == SourceKind::Window {
                                    "Share this window"
                                } else {
                                    "Share this screen"
                                }
                                .into(),
                            ),
                        )
                        .layout(Layout {
                            height: Length::Px(24.0),
                            ..Default::default()
                        }),
                    );
                    card.layout.height = Length::Px(224.0);
                }
                grid.children.push(card);
            }
            root.children.push(
                Element::new(7, Widget::Scroll)
                    .layout(Layout {
                        height: Length::Fill,
                        ..Default::default()
                    })
                    .children(vec![grid]),
            );
        }
        let mut buttons = Element::new(8, Widget::Panel)
            .without_surface()
            .class("shell-structure")
            .layout(Layout {
                flow: Flow::Row,
                height: Length::Px(44.0),
                gap: 12.0,
                ..Default::default()
            });
        if self.request.operation != Operation::Sharing {
            buttons
                .children
                .push(Element::new(CANCEL.0, Widget::Button("Cancel".into())));
        }
        let mut confirm = Element::new(
            CONFIRM.0,
            Widget::Button(
                match self.request.operation {
                    Operation::Share => {
                        if self.mode == Mode::Workspaces {
                            "Share selected workspaces"
                        } else {
                            "Share"
                        }
                    }
                    Operation::Screenshot => "Capture",
                    Operation::Sharing => "Stop sharing",
                }
                .into(),
            ),
        );
        confirm.enabled = self.request.operation == Operation::Sharing || !self.selected.is_empty();
        if self.direct_share() {
            buttons.children[0].layout.width = Length::Px(120.0);
            buttons.children.push(Element::new(
                9,
                Widget::Text(
                    if self.mode == Mode::Window {
                        "Click a window to start sharing"
                    } else {
                        "Click the screen to start sharing"
                    }
                    .into(),
                ),
            ));
        } else {
            buttons.children.push(confirm);
        }
        root.children.push(buttons);
        knave_shell_ui::styles::root(root.class("portal-picker"), self.theme)
    }

    fn sharing_control(&self) -> Element {
        let mut root = Element::new(0, Widget::Panel).layout(Layout {
            flow: Flow::Row,
            padding: 6.0,
            gap: 6.0,
            ..Default::default()
        });
        let app = if self.request.app_id.is_empty() {
            "Application"
        } else {
            &self.request.app_id
        };
        let workspace_numbers: Vec<_> = self
            .request
            .sources
            .iter()
            .filter_map(|source| {
                if let SourceKind::Workspace { workspace } = source.kind {
                    Some(workspace.to_string())
                } else {
                    None
                }
            })
            .collect();
        let text = if !workspace_numbers.is_empty() {
            format!(
                "{} · {}",
                if self.workspace_frozen {
                    "Frozen"
                } else {
                    "Sharing"
                },
                workspace_numbers.join(", ")
            )
        } else {
            format!(
                "Sharing · {}",
                self.request
                    .sources
                    .first()
                    .map_or(app, |source| source.name.as_str())
            )
        };
        let mut label = Element::new(4, Widget::Text(text)).layout(Layout {
            height: Length::Fill,
            ..Default::default()
        });
        label.style.text.wrap = TextWrap::None;
        label.classes.push(if self.workspace_frozen {
            "portal-frozen".into()
        } else {
            "portal-sharing".into()
        });
        root.children.push(label);
        root.children.push(
            Element::new(CONFIRM.0, Widget::Button("Stop sharing".into())).layout(Layout {
                width: Length::Px(104.0),
                height: Length::Fill,
                padding: 3.0,
                ..Default::default()
            }),
        );
        knave_shell_ui::styles::root(root.class("portal-picker"), self.theme)
    }
}
impl Application for Picker {
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
    fn uses_desktop(&self) -> bool {
        self.request.operation == Operation::Sharing
            && self
                .request
                .sources
                .iter()
                .any(|source| matches!(source.kind, SourceKind::Workspace { .. }))
    }
    fn desktop_snapshot(&mut self, snapshot: &knave_desktop_api::DesktopSnapshot) {
        if !self.uses_desktop() {
            return;
        }
        let active = snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.active)
            .map(|workspace| workspace.workspace.0);
        let frozen = !self.request.sources.iter().any(|source| matches!(source.kind, SourceKind::Workspace { workspace } if Some(workspace) == active));
        if frozen != self.workspace_frozen {
            self.workspace_frozen = frozen;
            self.dirty = true;
        }
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
                repeat: false,
                ..
            }
        ) && self.request.operation != Operation::Sharing
        {
            self.respond(Vec::new());
            return;
        }
        let result = self.scene.event(event);
        if let Some(Action::Activated(id)) = result.action {
            if let Some(index) = id.0.checked_sub(TAB).filter(|index| *index < 3) {
                self.mode = [Mode::Screen, Mode::Window, Mode::Workspaces][index as usize];
                self.selected.clear();
                self.dirty = true;
            } else if id == CANCEL {
                self.respond(Vec::new());
            } else if id == CONFIRM {
                if self.request.operation == Operation::Sharing || !self.selected.is_empty() {
                    self.respond(self.selected.iter().copied().collect());
                }
            } else if let Some(source) =
                id.0.checked_sub(SOURCE)
                    .and_then(|i| self.request.sources.get(i as usize))
            {
                let source_id = source.id;
                if self.direct_share() {
                    self.respond(vec![source_id]);
                    return;
                }
                if !self.selected.remove(&source_id) {
                    if !self.request.multiple && self.mode != Mode::Workspaces {
                        self.selected.clear();
                    }
                    self.selected.insert(source_id);
                }
                self.dirty = true;
            }
        }
    }
    fn needs_frame(&self) -> bool {
        self.dirty || self.scene.needs_frame()
    }
    fn frame(&mut self, size: [f32; 2], text: &mut dyn TextMeasurer) -> &DisplayList {
        if self.dirty || self.size != size {
            let focus = self.scene.focus();
            let user_theme_css = self.scene.user_theme_stylesheet().to_owned();
            let policy = self.scene.theme_policy();
            if let Ok(mut scene) = Scene::new(self.compose()) {
                scene
                    .set_theme_policy_stylesheets(
                        policy,
                        knave_shell_ui::styles::STYLESHEET,
                        &user_theme_css,
                    )
                    .expect("portal stylesheet is valid");
                scene.layout(size, text);
                let initial = (self.request.operation != Operation::Sharing).then_some(CANCEL);
                let _ = scene.set_focus(focus.or(initial));
                self.scene = scene;
            }
            self.size = size;
            self.dirty = false;
        }
        self.scene.display_list(size, text)
    }
    fn should_close(&self) -> bool {
        self.closed
    }
}
fn surface_options(sharing: bool) -> SurfaceOptions {
    SurfaceOptions {
        size: Some(if sharing { [350, 36] } else { [820, 640] }),
        position: if sharing {
            SurfacePosition::TopRight
        } else {
            SurfacePosition::Center
        },
        keyboard: if sharing {
            KeyboardMode::None
        } else {
            KeyboardMode::Exclusive
        },
        layer: SurfaceLayer::Overlay,
        namespace: if sharing {
            "knave-portal-sharing"
        } else {
            "knave-portal-picker"
        }
        .into(),
    }
}

pub fn run() -> Result<(), String> {
    run_with_theme(Theme::Dark)
}

pub fn run_with_theme(theme: Theme) -> Result<(), String> {
    let mut bytes = Vec::new();
    io::stdin()
        .take(MAX_REQUEST as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_REQUEST {
        return Err("picker request exceeds budget".into());
    }
    let request: Request = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let sharing = request.operation == Operation::Sharing;
    knave_wayland::run_application(
        surface_options(sharing),
        Picker::with_theme(request, theme)?,
    )
    .map_err(|e| e.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> Request {
        Request {
            request_id: "test".into(),
            app_id: "app".into(),
            operation: Operation::Share,
            multiple: false,
            sources: vec![knave_portal_api::Source {
                id: 3,
                name: "monitor".into(),
                description: "".into(),
                width: 1920,
                height: 1080,
                preview_png: None,
                kind: SourceKind::Monitor,
            }],
        }
    }
    #[test]
    fn share_starts_disabled_and_requires_explicit_selection() {
        let mut req = request();
        req.multiple = true;
        let mut picker = Picker::new(req).unwrap();
        assert!(picker.selected.is_empty());
        let root = picker.compose();
        assert!(
            !root
                .children
                .last()
                .unwrap()
                .children
                .last()
                .unwrap()
                .enabled
        );
        picker.selected.insert(3);
        assert!(
            picker
                .compose()
                .children
                .last()
                .unwrap()
                .children
                .last()
                .unwrap()
                .enabled
        );
    }
    struct Metrics;
    impl TextMeasurer for Metrics {
        fn measure(&mut self, _: &str, style: &knave_ui::TextStyle, width: f32) -> TextLayout {
            TextLayout {
                size: [width.min(160.0), style.line_height],
                carets: Vec::new(),
            }
        }
        fn measure_spans(
            &mut self,
            _: &[knave_ui::TextSpan],
            style: &knave_ui::TextStyle,
            width: f32,
        ) -> TextLayout {
            self.measure("", style, width)
        }
    }
    fn click(picker: &mut Picker, id: ElementId) {
        let rect = picker.scene.bounds(id).unwrap();
        let point = [rect.x + rect.width / 2.0, rect.y + rect.height / 2.0];
        picker.input(Input::PointerMove(point));
        picker.input(Input::PointerDown(point));
        // A frame between press and release must retain toolkit pointer capture.
        picker.frame(picker.size, &mut Metrics);
        picker.input(Input::PointerUp(point));
    }
    #[test]
    fn actual_pointer_selection_toggles_and_confirmation_closes() {
        let mut req = request();
        req.multiple = true;
        let mut picker = Picker::new(req).unwrap();
        picker.frame([820.0, 640.0], &mut Metrics);
        click(&mut picker, ElementId(SOURCE));
        assert_eq!(picker.selected.iter().copied().collect::<Vec<_>>(), vec![3]);
        picker.frame([820.0, 640.0], &mut Metrics);
        click(&mut picker, ElementId(SOURCE));
        assert!(picker.selected.is_empty());
        picker.frame([820.0, 640.0], &mut Metrics);
        click(&mut picker, ElementId(SOURCE));
        picker.frame([820.0, 640.0], &mut Metrics);
        click(&mut picker, CONFIRM);
        assert!(picker.should_close());
    }
    #[test]
    fn portal_surfaces_and_selection_follow_the_selected_theme() {
        let mut picker = Picker::with_theme(request(), Theme::Light).unwrap();
        picker.frame([820.0, 640.0], &mut Metrics);

        let mut defaults = Scene::new(Element::new(0, Widget::Panel)).unwrap();
        defaults.set_theme(Theme::Light).unwrap();
        assert_eq!(
            picker
                .scene
                .computed_style(ElementId(0))
                .unwrap()
                .style
                .background,
            defaults
                .computed_style(ElementId(0))
                .unwrap()
                .style
                .background
        );
        defaults
            .set_stylesheet("panel { background-color: var(--surface-selected); }")
            .unwrap();
        // Only the selection fill is shared; buttons keep their themed elevation.
        assert_eq!(
            picker
                .scene
                .computed_style(ElementId(TAB))
                .unwrap()
                .style
                .background
                .and_then(|paint| paint.fill),
            defaults
                .computed_style(ElementId(0))
                .unwrap()
                .style
                .background
                .and_then(|paint| paint.fill)
        );
    }
    #[test]
    fn sharing_stop_button_closes_without_source_selection() {
        let mut req = request();
        req.operation = Operation::Sharing;
        let mut picker = Picker::new(req).unwrap();
        picker.frame([350.0, 36.0], &mut Metrics);
        click(&mut picker, CONFIRM);
        assert!(picker.should_close());
    }
    #[test]
    fn single_share_requires_one_explicit_source_activation() {
        let mut picker = Picker::new(request()).unwrap();
        picker.frame([820.0, 640.0], &mut Metrics);
        assert!(picker.selected.is_empty());
        assert!(!picker.should_close());
        assert!(picker.scene.bounds(CONFIRM).is_none());
        click(&mut picker, ElementId(SOURCE));
        assert!(picker.should_close());
    }

    #[test]
    fn arbitrary_workspace_set_requires_confirmation_and_mode_switch_clears_it() {
        let mut req = request();
        for workspace in [1, 2, 3, 9] {
            let mut source = req.sources[0].clone();
            source.id = 100 + workspace;
            source.kind = SourceKind::Workspace { workspace };
            source.name = format!("Workspace {workspace}");
            req.sources.push(source);
        }
        let mut picker = Picker::new(req).unwrap();
        picker.frame([820.0, 640.0], &mut Metrics);
        click(&mut picker, ElementId(TAB + 2));
        picker.frame([820.0, 640.0], &mut Metrics);
        for index in 1..=4 {
            click(&mut picker, ElementId(SOURCE + index));
            picker.frame([820.0, 640.0], &mut Metrics);
        }
        assert_eq!(picker.selected, BTreeSet::from([101, 102, 103, 109]));
        assert!(!picker.should_close());
        click(&mut picker, ElementId(TAB));
        picker.frame([820.0, 640.0], &mut Metrics);
        assert!(picker.selected.is_empty());
        click(&mut picker, ElementId(TAB + 2));
        picker.frame([820.0, 640.0], &mut Metrics);
        click(&mut picker, ElementId(SOURCE + 4));
        picker.frame([820.0, 640.0], &mut Metrics);
        click(&mut picker, CONFIRM);
        assert!(picker.should_close());
    }

    #[test]
    fn window_card_is_one_explicit_activation_without_selecting_a_screen() {
        let mut req = request();
        let mut source = req.sources[0].clone();
        source.id = 7;
        source.kind = SourceKind::Window;
        req.sources.push(source);
        let mut picker = Picker::new(req).unwrap();
        picker.frame([820.0, 640.0], &mut Metrics);
        click(&mut picker, ElementId(TAB + 1));
        picker.frame([820.0, 640.0], &mut Metrics);
        assert!(picker.scene.bounds(ElementId(SOURCE)).is_none());
        click(&mut picker, ElementId(SOURCE + 1));
        assert!(picker.should_close());
    }

    #[test]
    fn many_windows_keep_cancel_visible_and_scroll_sources() {
        let mut req = request();
        req.sources = (1..=20)
            .map(|id| {
                let mut source = req.sources[0].clone();
                source.id = id;
                source.kind = SourceKind::Window;
                source
            })
            .collect();
        let mut picker = Picker::new(req).unwrap();
        picker.frame([820.0, 640.0], &mut Metrics);
        let cancel = picker.scene.bounds(CANCEL).unwrap();
        let scroll = picker.scene.bounds(ElementId(7)).unwrap();
        assert!(cancel.y + cancel.height <= 640.0);
        assert!(scroll.y + scroll.height <= cancel.y);
        click(&mut picker, CANCEL);
        assert!(picker.should_close());
    }

    #[test]
    fn screenshot_keeps_selection_and_confirmation() {
        let mut req = request();
        req.operation = Operation::Screenshot;
        let mut picker = Picker::new(req).unwrap();
        picker.frame([820.0, 640.0], &mut Metrics);
        click(&mut picker, ElementId(SOURCE));
        assert!(!picker.should_close());
        assert!(picker.selected.contains(&3));
        picker.frame([820.0, 640.0], &mut Metrics);
        click(&mut picker, CONFIRM);
        assert!(picker.should_close());
    }

    #[test]
    fn sharing_control_is_small_and_never_requests_keyboard_focus() {
        let options = surface_options(true);
        assert_eq!(options.size, Some([350, 36]));
        assert_eq!(options.position, SurfacePosition::TopRight);
        assert_eq!(options.keyboard, KeyboardMode::None);
    }
    #[test]
    fn escape_cancels() {
        let mut picker = Picker::new(request()).unwrap();
        picker.input(Input::Key {
            key: Key::Escape,
            pressed: true,
            repeat: false,
            modifiers: KeyModifiers::default(),
        });
        assert!(picker.should_close());
    }
}
