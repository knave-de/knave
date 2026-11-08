//! Native key capture UI for applications configuring global shortcuts.
use knave_portal_api::{MAX_REQUEST, ShortcutAssignmentReply, ShortcutAssignmentRequest};
use knave_ui::{DisplayList, toolkit::*};
use knave_wayland::{Application, KeyboardMode, SurfaceLayer, SurfaceOptions, SurfacePosition};
use std::io::{self, Read};

const CANCEL: ElementId = ElementId(2);
const SHORTCUT_BASE: u64 = 100;
pub struct Assignment {
    request: ShortcutAssignmentRequest,
    scene: Scene,
    selected: Option<usize>,
    reply: Option<ShortcutAssignmentReply>,
    dirty: bool,
    size: [f32; 2],
}
impl Assignment {
    fn new(request: ShortcutAssignmentRequest) -> Result<Self, String> {
        request.validate()?;
        let scene = Scene::new(Self::compose(&request, None, request.error.as_deref()))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            request,
            scene,
            selected: None,
            reply: None,
            dirty: true,
            size: [0.0; 2],
        })
    }
    fn compose(
        request: &ShortcutAssignmentRequest,
        selected: Option<usize>,
        error: Option<&str>,
    ) -> Element {
        let mut root = Element::new(0, Widget::Panel).layout(Layout {
            padding: 24.0,
            gap: 12.0,
            ..Default::default()
        });
        root.children.push(
            Element::new(10, Widget::Text("Configure application shortcuts".into())).layout(
                Layout {
                    height: Length::Px(34.0),
                    ..Default::default()
                },
            ),
        );
        root.children.push(
            Element::new(
                11,
                Widget::Text(format!("{} requested these shortcuts", request.app_id)),
            )
            .layout(Layout {
                height: Length::Px(28.0),
                ..Default::default()
            }),
        );
        if let Some(error) = error {
            root.children.push(
                Element::new(12, Widget::Text(error.to_owned()))
                    .class("portal-error")
                    .layout(Layout {
                        height: Length::Px(38.0),
                        ..Default::default()
                    }),
            );
        }
        let mut list = Element::new(13, Widget::Scroll).layout(Layout {
            height: Length::Fill,
            gap: 8.0,
            ..Default::default()
        });
        for (index, shortcut) in request.shortcuts.iter().enumerate() {
            let trigger = shortcut.trigger.as_deref().unwrap_or("Unassigned");
            let selected_label = if selected == Some(index) {
                " · Press a key chord"
            } else {
                ""
            };
            let mut row = Element::new(
                SHORTCUT_BASE + index as u64,
                Widget::Button(format!(
                    "{}  —  {}{}",
                    shortcut.description, trigger, selected_label
                )),
            )
            .layout(Layout {
                height: Length::Px(48.0),
                ..Default::default()
            });
            if selected == Some(index) {
                row.classes.push("portal-selected".into());
            }
            list.children.push(row);
        }
        root.children.push(list);
        root.children.push(
            Element::new(14, Widget::Text(
                "Choose an action, then press a key with Ctrl, Alt, Shift or Super. Escape cancels.".into(),
            )).layout(Layout { height: Length::Px(38.0), ..Default::default() }),
        );
        let mut actions = Element::new(15, Widget::Panel)
            .class("shell-structure")
            .layout(Layout {
                flow: Flow::Row,
                height: Length::Px(44.0),
                gap: 8.0,
                ..Default::default()
            });
        actions
            .children
            .push(Element::new(CANCEL.0, Widget::Button("Cancel".into())));
        root.children.push(actions);
        root
    }
    fn finish(&mut self, shortcut_id: Option<String>, trigger: Option<String>) {
        if self.reply.is_some() {
            return;
        }
        let reply = ShortcutAssignmentReply {
            request_id: self.request.request_id.clone(),
            shortcut_id,
            trigger,
        };
        if let Ok(json) = serde_json::to_string(&reply) {
            println!("{json}");
        }
        self.reply = Some(reply);
    }
    fn error(&self) -> Option<&str> {
        self.request.error.as_deref()
    }
}
impl Application for Assignment {
    fn wants_raw_keys(&self) -> bool {
        true
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
        ) {
            self.finish(None, None);
            return;
        }
        if let Some(Action::Activated(id)) = self.scene.event(event).action {
            if id == CANCEL {
                self.finish(None, None);
            } else if let Some(index) =
                id.0.checked_sub(SHORTCUT_BASE)
                    .filter(|index| (*index as usize) < self.request.shortcuts.len())
            {
                self.selected = Some(index as usize);
                self.dirty = true;
            }
        }
    }
    fn raw_key(&mut self, keysym: &str, pressed: bool, repeat: bool, modifiers: KeyModifiers) {
        if !pressed
            || repeat
            || self.reply.is_some()
            || keysym.is_empty()
            || matches!(
                keysym,
                "Escape"
                    | "Shift_L"
                    | "Shift_R"
                    | "Control_L"
                    | "Control_R"
                    | "Alt_L"
                    | "Alt_R"
                    | "Super_L"
                    | "Super_R"
                    | "Meta_L"
                    | "Meta_R"
                    | "Hyper_L"
                    | "Hyper_R"
                    | "ISO_Level3_Shift"
                    | "NoSymbol"
            )
            || !(modifiers.control || modifiers.alt || modifiers.shift || modifiers.logo)
        {
            return;
        }
        let Some(index) = self.selected else {
            return;
        };
        let mut parts = Vec::with_capacity(5);
        if modifiers.control {
            parts.push("CTRL".to_owned());
        }
        if modifiers.alt {
            parts.push("ALT".to_owned());
        }
        if modifiers.shift {
            parts.push("SHIFT".to_owned());
        }
        if modifiers.logo {
            parts.push("SUPER".to_owned());
        }
        parts.push(keysym.to_ascii_uppercase());
        self.finish(
            Some(self.request.shortcuts[index].id.clone()),
            Some(parts.join("+")),
        );
    }
    fn needs_frame(&self) -> bool {
        self.dirty || self.scene.needs_frame()
    }
    fn frame(&mut self, size: [f32; 2], text: &mut dyn TextMeasurer) -> &DisplayList {
        if self.dirty || self.size != size {
            let focus = self.scene.focus();
            if let Ok(mut scene) =
                Scene::new(Self::compose(&self.request, self.selected, self.error()))
            {
                scene.layout(size, text);
                let _ = scene.set_focus(focus.or(Some(CANCEL)));
                self.scene = scene;
            }
            self.size = size;
            self.dirty = false;
        }
        self.scene.display_list(size, text)
    }
    fn should_close(&self) -> bool {
        self.reply.is_some()
    }
}

pub fn run_with_theme(theme: Theme) -> Result<(), String> {
    let mut bytes = Vec::new();
    io::stdin()
        .take(MAX_REQUEST as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_REQUEST {
        return Err("shortcut assignment request exceeds budget".into());
    }
    let request: ShortcutAssignmentRequest =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    request.validate()?;
    knave_wayland::run_application_with_theme(
        SurfaceOptions {
            size: Some([680, 560]),
            position: SurfacePosition::Center,
            keyboard: KeyboardMode::Exclusive,
            layer: SurfaceLayer::Overlay,
            namespace: "knave-shortcut-assignment".into(),
        },
        Assignment::new(request)?,
        theme,
    )
    .map_err(|error| error.to_string())
}
