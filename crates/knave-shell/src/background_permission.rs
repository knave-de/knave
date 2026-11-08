//! User consent UI for applications requesting background activity.
use knave_portal_api::{BackgroundChoice, BackgroundReply, BackgroundRequest, MAX_REQUEST};
use knave_ui::{DisplayList, TextWrap, toolkit::*};
use knave_wayland::{Application, KeyboardMode, SurfaceLayer, SurfaceOptions, SurfacePosition};
use std::io::{self, Read};

const DENY: ElementId = ElementId(20);
const ALLOW: ElementId = ElementId(21);
const ALLOW_ONCE: ElementId = ElementId(22);

struct Prompt {
    request: BackgroundRequest,
    theme: Theme,
    scene: Scene,
    reply: Option<BackgroundReply>,
    dirty: bool,
    size: [f32; 2],
}

impl Prompt {
    /// Validate a background request and prepare its themed prompt scene.
    fn new(request: BackgroundRequest, theme: Theme) -> Result<Self, String> {
        request.validate()?;
        let mut scene = Scene::new(knave_shell_ui::styles::root(Self::compose(&request), theme))
            .map_err(|error| error.to_string())?;
        scene
            .set_theme_stylesheets(theme, knave_shell_ui::styles::STYLESHEET, "")
            .map_err(|error| error.to_string())?;
        let _ = scene.set_focus(Some(DENY));
        Ok(Self {
            request,
            theme,
            scene,
            reply: None,
            dirty: true,
            size: [0.0; 2],
        })
    }

    /// Lay out the identity, reason, and fixed action controls for the request.
    fn compose(request: &BackgroundRequest) -> Element {
        let mut root = Element::new(0, Widget::Panel).layout(Layout {
            padding: 24.0,
            gap: 12.0,
            ..Default::default()
        });
        root.children.push(
            Element::new(1, Widget::Text("Allow background activity?".into())).layout(Layout {
                height: Length::Px(34.0),
                ..Default::default()
            }),
        );
        root.children.push(
            Element::new(2, Widget::Text(request.app_id.clone())).layout(Layout {
                height: Length::Px(24.0),
                ..Default::default()
            }),
        );
        root.children.push(
            Element::new(5, Widget::Text(request.name.clone())).layout(Layout {
                height: Length::Px(24.0),
                ..Default::default()
            }),
        );
        if !request.reason.is_empty() {
            let mut reason = Element::new(3, Widget::Scroll).layout(Layout {
                height: Length::Fill,
                ..Default::default()
            });
            let mut text = Element::new(31, Widget::Text(request.reason.clone()));
            text.style.text.wrap = TextWrap::WordOrGlyph;
            reason.children.push(text);
            root.children.push(reason);
        }
        let mut actions = Element::new(4, Widget::Panel)
            .class("shell-structure")
            .layout(Layout {
                flow: Flow::Row,
                height: Length::Px(48.0),
                gap: 8.0,
                ..Default::default()
            });
        actions
            .children
            .push(Element::new(DENY.0, Widget::Button("Deny".into())));
        actions
            .children
            .push(Element::new(ALLOW.0, Widget::Button("Allow".into())));
        actions.children.push(Element::new(
            ALLOW_ONCE.0,
            Widget::Button("Allow this time".into()),
        ));
        root.children.push(actions);
        root
    }

    /// Emit one reply for the active request and close the prompt.
    fn finish(&mut self, choice: BackgroundChoice) {
        if self.reply.is_some() {
            return;
        }
        let reply = BackgroundReply {
            request_id: self.request.request_id.clone(),
            choice,
        };
        if let Ok(json) = serde_json::to_string(&reply) {
            println!("{json}");
        }
        self.reply = Some(reply);
    }
}

impl Application for Prompt {
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
            self.finish(BackgroundChoice::Deny);
            return;
        }
        if let Some(Action::Activated(id)) = self.scene.event(event).action {
            match id {
                DENY => self.finish(BackgroundChoice::Deny),
                ALLOW => self.finish(BackgroundChoice::Allow),
                ALLOW_ONCE => self.finish(BackgroundChoice::AllowOnce),
                _ => {}
            }
        }
    }

    fn needs_frame(&self) -> bool {
        self.dirty || self.scene.needs_frame()
    }

    /// Render the prompt at its current size, rebuilding the themed scene as needed.
    fn frame(&mut self, size: [f32; 2], text: &mut dyn TextMeasurer) -> &DisplayList {
        if self.dirty || self.size != size {
            if let Ok(mut scene) = Scene::new(knave_shell_ui::styles::root(
                Self::compose(&self.request),
                self.theme,
            )) {
                if scene
                    .set_theme_stylesheets(self.theme, knave_shell_ui::styles::STYLESHEET, "")
                    .is_err()
                {
                    return self.scene.display_list(size, text);
                }
                scene.layout(size, text);
                let _ = scene.set_focus(Some(DENY));
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

/// Read one bounded background request and show its themed consent prompt.
pub fn run_with_theme(theme: Theme) -> Result<(), String> {
    let mut bytes = Vec::new();
    io::stdin()
        .take(MAX_REQUEST as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_REQUEST {
        return Err("background request exceeds budget".into());
    }
    let request: BackgroundRequest =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    request.validate()?;
    knave_wayland::run_application_with_theme(
        SurfaceOptions {
            size: Some([620, 260]),
            position: SurfacePosition::Center,
            keyboard: KeyboardMode::Exclusive,
            layer: SurfaceLayer::Overlay,
            namespace: "knave-background-permission".into(),
        },
        Prompt::new(request, theme)?,
        theme,
    )
    .map_err(|error| error.to_string())
}
