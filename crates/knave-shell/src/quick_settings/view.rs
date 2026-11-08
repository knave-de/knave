use super::*;
use knave_ui::toolkit::{ButtonVariant, ComponentKind};
use knave_ui::{ShapePaint, TextWrap, UiImage};
use std::sync::{Mutex, OnceLock};

struct Palette;
impl Palette {
    fn new() -> Self {
        Self
    }
    fn element(&self, id: u64, w: Widget) -> Element {
        let class = match &w {
            Widget::Panel => Some(("shell-structure", "quick-settings-structure")),
            Widget::Button(_) | Widget::Menu { .. } => {
                Some(("shell-control", "quick-settings-button"))
            }
            Widget::Slider(_) => Some(("shell-control", "quick-settings-slider")),
            Widget::Text(_) => Some(("shell-text", "quick-settings-text")),
            _ => None,
        };
        let mut e = Element::new(id, w);
        if let Some((shared, local)) = class {
            e = e.class(shared).class(local);
        }
        e.style.text.wrap = TextWrap::WordOrGlyph;
        e
    }
    fn text(&self, id: u64, s: impl Into<String>, muted: bool) -> Element {
        let mut e = if muted {
            Element::subtitle(id, s)
                .class("shell-muted")
                .class("quick-settings-muted")
        } else {
            self.element(id, Widget::Text(s.into()))
        };
        e.style.text.wrap = TextWrap::WordOrGlyph;
        e
    }
    fn panel(&self, id: u64) -> Element {
        let mut e = self.element(id, Widget::Panel).without_surface();
        e.style.background = Some(ShapePaint::fill(Color::rgba(0, 0, 0, 0)));
        e
    }
    fn surface_panel(&self, id: u64) -> Element {
        let mut e = Element::new(id, Widget::Panel);
        e.style.text.wrap = TextWrap::WordOrGlyph;
        e
    }
    fn button(&self, id: u64, label: impl Into<String>, enabled: bool) -> Element {
        let mut e = self.element(id, Widget::Button(label.into()));
        e.layout.padding = 8.0;
        e.layout.height = Length::Px(44.0);
        e.enabled = enabled;
        e
    }
    fn header_button(&self, id: u64, label: &str, width: f32) -> Element {
        let mut e = self.button(id, label, true);
        e.layout.width = Length::Px(width);
        e.layout.height = Length::Px(30.0);
        e.layout.padding = 0.0;
        e.layout.padding_edges = Some(Insets::all(0.0));
        e.layout.align = Align::Center;
        e.class("quick-settings-header-button")
    }
    fn slider(&self, id: u64, min: f64, max: f64, value: f64, step: f64, enabled: bool) -> Element {
        let mut e = self.element(
            id,
            Widget::Slider(Slider::new(min, max, value, Some(step)).unwrap()),
        );
        e.enabled = enabled;
        e.layout.height = Length::Px(30.0);
        e
    }
}
fn clock(minute: u16) -> String {
    format!("{:02}:{:02}", minute / 60, minute % 60)
}

fn profile_name(profile: &str) -> &str {
    match profile {
        "power-saver" => "Power saver",
        "balanced" => "Balanced",
        "performance" => "Performance",
        other => other,
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Icon {
    Power,
    Close,
    Network,
    Bluetooth,
    Speaker,
    Microphone,
    Brightness,
    Battery,
}

fn raster_icon(icon: Icon, color: Color, battery_percent: Option<u8>, charging: bool) -> UiImage {
    const SIZE: usize = 24;
    let mut pixels = vec![0; SIZE * SIZE * 4];
    let mut segments: Vec<([f32; 2], [f32; 2])> = Vec::new();
    let mut add_path = |points: &[[f32; 2]]| {
        segments.extend(points.windows(2).map(|w| (w[0], w[1])));
    };
    match icon {
        Icon::Power => {
            let arc: Vec<_> = (0..=28)
                .map(|i| {
                    let a = (-45.0 + (270.0 * i as f32 / 28.0)).to_radians();
                    [12.0 + 8.5 * a.cos(), 12.0 + 8.5 * a.sin()]
                })
                .collect();
            add_path(&arc);
            segments.push(([12.0, 3.0], [12.0, 12.0]));
        }
        Icon::Close => {
            segments.push(([6.0, 6.0], [18.0, 18.0]));
            segments.push(([18.0, 6.0], [6.0, 18.0]));
        }
        Icon::Network => {
            add_path(&[
                [4.0, 9.0],
                [7.0, 6.5],
                [10.0, 5.0],
                [12.0, 4.5],
                [14.0, 5.0],
                [17.0, 6.5],
                [20.0, 9.0],
            ]);
            add_path(&[
                [7.0, 12.0],
                [9.0, 10.5],
                [12.0, 9.5],
                [15.0, 10.5],
                [17.0, 12.0],
            ]);
            add_path(&[[10.0, 15.0], [12.0, 14.0], [14.0, 15.0]]);
            segments.push(([12.0, 18.0], [12.0, 18.2]));
        }
        Icon::Bluetooth => {
            add_path(&[
                [7.0, 7.0],
                [17.0, 17.0],
                [12.0, 22.0],
                [12.0, 2.0],
                [17.0, 7.0],
                [7.0, 17.0],
            ]);
        }
        Icon::Speaker => {
            add_path(&[
                [4.0, 9.0],
                [8.0, 9.0],
                [13.0, 5.0],
                [13.0, 19.0],
                [8.0, 15.0],
                [4.0, 15.0],
                [4.0, 9.0],
            ]);
            add_path(&[[16.0, 9.0], [18.0, 12.0], [16.0, 15.0]]);
            add_path(&[[18.0, 6.0], [21.0, 12.0], [18.0, 18.0]]);
        }
        Icon::Microphone => {
            add_path(&[
                [9.0, 6.0],
                [9.0, 5.0],
                [10.0, 4.0],
                [14.0, 4.0],
                [15.0, 5.0],
                [15.0, 12.0],
                [14.0, 14.0],
                [10.0, 14.0],
                [9.0, 12.0],
                [9.0, 6.0],
            ]);
            add_path(&[
                [6.0, 10.0],
                [6.0, 14.0],
                [8.0, 17.0],
                [12.0, 19.0],
                [16.0, 17.0],
                [18.0, 14.0],
                [18.0, 10.0],
            ]);
            add_path(&[[12.0, 19.0], [12.0, 22.0], [9.0, 22.0]]);
        }
        Icon::Brightness => {
            add_path(&[[12.0, 4.0], [12.0, 6.0]]);
            add_path(&[[12.0, 18.0], [12.0, 20.0]]);
            add_path(&[[4.0, 12.0], [6.0, 12.0]]);
            add_path(&[[18.0, 12.0], [20.0, 12.0]]);
            add_path(&[[6.3, 6.3], [7.8, 7.8]]);
            add_path(&[[16.2, 16.2], [17.7, 17.7]]);
            add_path(&[[17.7, 6.3], [16.2, 7.8]]);
            add_path(&[[7.8, 16.2], [6.3, 17.7]]);
            for i in 0..32 {
                let a = std::f32::consts::TAU * i as f32 / 32.0;
                segments.push((
                    [12.0 + 4.0 * a.cos(), 12.0 + 4.0 * a.sin()],
                    [12.0 + 4.0 * (a + 0.2).cos(), 12.0 + 4.0 * (a + 0.2).sin()],
                ));
            }
        }
        Icon::Battery => {
            add_path(&[
                [4.0, 7.0],
                [18.0, 7.0],
                [18.0, 9.0],
                [20.0, 9.0],
                [20.0, 15.0],
                [18.0, 15.0],
                [18.0, 17.0],
                [4.0, 17.0],
                [4.0, 7.0],
            ]);
        }
    }
    const SAMPLES: usize = 4;
    let bolt = [
        [13.0, 8.5],
        [10.5, 12.0],
        [12.5, 12.0],
        [11.0, 15.5],
        [15.0, 11.0],
        [13.0, 11.0],
        [14.0, 8.5],
    ];
    let bolt_segments: Vec<_> = if icon == Icon::Battery && charging {
        bolt.windows(2).map(|w| (w[0], w[1])).collect()
    } else {
        Vec::new()
    };
    for y in 0..SIZE {
        for x in 0..SIZE {
            let mut coverage = 0u16;
            let mut fill = 0u16;
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let point = [
                        x as f32 + (sx as f32 + 0.5) / SAMPLES as f32,
                        y as f32 + (sy as f32 + 0.5) / SAMPLES as f32,
                    ];
                    let in_battery_fill = icon == Icon::Battery
                        && battery_percent.is_some_and(|percent| {
                            point[0] >= 5.5
                                && point[0] <= 5.5 + battery_fill_width(percent)
                                && point[1] >= 8.5
                                && point[1] <= 15.5
                        });
                    let bolt_pixel = bolt_segments
                        .iter()
                        .any(|(a, b)| distance(point, *a, *b) <= 0.8);
                    fill += u16::from(in_battery_fill && !bolt_pixel);
                    coverage += u16::from(
                        segments.iter().any(|(a, b)| distance(point, *a, *b) <= 0.8)
                            || (bolt_pixel && !in_battery_fill),
                    );
                }
            }
            let coverage = coverage as f32 / (SAMPLES * SAMPLES) as f32;
            let fill = fill as f32 / (SAMPLES * SAMPLES) as f32;
            let alpha = (coverage + fill).min(1.0);
            if alpha > 0.0 {
                let i = (y * SIZE + x) * 4;
                pixels[i..i + 4].copy_from_slice(&[
                    color.red,
                    color.green,
                    color.blue,
                    (color.alpha as f32 * alpha).round() as u8,
                ]);
            }
        }
    }
    UiImage::from_rgba(SIZE as u32, SIZE as u32, pixels).expect("icon dimensions are valid")
}

fn battery_fill_width(percent: u8) -> f32 {
    11.0 * f32::from(percent.min(100)) / 100.0
}

fn is_charging(status: &str) -> bool {
    status.eq_ignore_ascii_case("charging")
}

pub(super) fn text_color(
    policy: ThemePolicy,
    user_theme_css: &str,
) -> Result<Color, StylesheetError> {
    let mut scene = Scene::new(knave_shell_ui::styles::root(
        Element::new(
            1,
            Widget::Icon(knave_ui::toolkit::Icon {
                image: UiImage::from_rgba(1, 1, vec![0; 4]).expect("transparent sample icon"),
                name: "quick settings".into(),
            }),
        )
        .class("quick-settings-icon"),
        policy.scheme,
    ))
    .expect("icon text style scene is valid");
    scene.set_theme_policy_stylesheets(
        policy,
        knave_shell_ui::styles::STYLESHEET,
        user_theme_css,
    )?;
    Ok(scene
        .computed_style(ElementId(1))
        .map(|style| style.style.text.color)
        .unwrap_or(Color::TEXT))
}

fn color_key(color: Color) -> [u8; 4] {
    [color.red, color.green, color.blue, color.alpha]
}

fn icon_image(icon: Icon, color: Color) -> UiImage {
    type IconCache = Mutex<std::collections::HashMap<([u8; 4], usize), UiImage>>;
    static CACHE: OnceLock<IconCache> = OnceLock::new();
    let key = (color_key(color), icon as usize);
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !cache.contains_key(&key) && cache.len() >= 128 {
        cache.clear();
    }
    cache
        .entry(key)
        .or_insert_with(|| raster_icon(icon, color, None, false))
        .clone()
}

type BatteryIconCache = Mutex<std::collections::HashMap<(u8, bool, [u8; 4]), UiImage>>;

fn battery_image(percent: u8, status: &str, color: Color) -> UiImage {
    // Bound cached raster memory across user-theme color changes.
    static CACHE: OnceLock<BatteryIconCache> = OnceLock::new();
    let key = (percent.min(100), is_charging(status), color_key(color));
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !cache.contains_key(&key) && cache.len() >= 256 {
        cache.clear();
    }
    cache
        .entry(key)
        .or_insert_with(|| raster_icon(Icon::Battery, color, Some(key.0), key.1))
        .clone()
}

fn distance(point: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let delta = [b[0] - a[0], b[1] - a[1]];
    let length = delta[0] * delta[0] + delta[1] * delta[1];
    let t = if length == 0.0 {
        0.0
    } else {
        (((point[0] - a[0]) * delta[0] + (point[1] - a[1]) * delta[1]) / length).clamp(0.0, 1.0)
    };
    let dx = point[0] - (a[0] + t * delta[0]);
    let dy = point[1] - (a[1] + t * delta[1]);
    (dx * dx + dy * dy).sqrt()
}
impl QuickSettings {
    fn id(&mut self) -> u64 {
        let id = self.next;
        self.next += 1;
        id
    }
    fn label(&mut self, p: &Palette, s: impl Into<String>, muted: bool) -> Element {
        let id = self.id();
        p.text(id, s, muted)
    }
    fn button(
        &mut self,
        p: &Palette,
        s: impl Into<String>,
        target: Target,
        enabled: bool,
    ) -> Element {
        let id = self.id();
        let label = s.into();
        self.targets.insert(ElementId(id), target);
        let mut e = p.button(id, label.clone(), enabled);
        if label.contains('\n') {
            e.layout.height = Length::Px(56.0);
        }
        e
    }
    fn group(&mut self, p: &Palette) -> Element {
        let id = self.id();
        let mut group = p
            .surface_panel(id)
            .component(ComponentKind::Card)
            .layout(Layout {
                padding: 12.0,
                gap: 8.0,
                ..Default::default()
            });
        group = group.class("quick-settings-card");
        group.style.background = None;
        group
    }
    fn stack(&mut self, p: &Palette) -> Element {
        let id = self.id();
        p.panel(id).layout(Layout {
            gap: 8.0,
            ..Default::default()
        })
    }
    fn row(&mut self, p: &Palette) -> Element {
        let mut e = self.stack(p);
        e.layout.flow = Flow::Row;
        e.layout.align = Align::Center;
        e
    }
    fn icon(&mut self, icon: Icon) -> Element {
        Element::new(
            self.id(),
            Widget::Image(icon_image(icon, self.icon_color), Default::default()),
        )
        .layout(Layout {
            width: Length::Px(18.0),
            height: Length::Px(18.0),
            ..Default::default()
        })
    }
    fn battery_icon(&mut self, percent: u8, status: &str) -> Element {
        Element::new(
            self.id(),
            Widget::Image(
                battery_image(percent, status, self.icon_color),
                Default::default(),
            ),
        )
        .layout(Layout {
            width: Length::Px(18.0),
            height: Length::Px(18.0),
            ..Default::default()
        })
    }
    fn icon_control(&mut self, mut control: Element, icon: Icon, label: &str) -> Element {
        control.layout.flow = Flow::Overlay;
        control.layout.padding = 6.0;
        control.layout.padding_edges = None;
        control.button_variant = Some(ButtonVariant::Ghost);
        control = control.class("quick-settings-icon-control");
        control.tooltip = Some(label.into());
        control.children.push(self.icon(icon));
        control
    }
    fn separator(&mut self) -> Element {
        Element::new(self.id(), Widget::Separator)
            .class("quick-settings-separator")
            .layout(Layout {
                height: Length::Px(1.0),
                ..Default::default()
            })
    }
    fn tile(
        &mut self,
        p: &Palette,
        icon: Icon,
        title: &str,
        subtitle: String,
        target: Target,
        state: (bool, bool),
    ) -> Element {
        let (enabled, active) = state;
        let mut b = self.button(p, "", target, enabled);
        b.tooltip = Some(format!("{title}: {subtitle}"));
        b.layout.flow = Flow::Row;
        b.layout.align = Align::Center;
        b.layout.gap = 8.0;
        b.layout.height = Length::Px(64.0);
        b = b.class("quick-settings-tile");
        b.children.push(self.icon(icon));
        let mut copy = self.stack(p);
        copy.layout.gap = 2.0;
        copy.children.push(self.label(p, title, false));
        let mut sub = self.label(p, subtitle, true);
        sub.style.text.wrap = TextWrap::None;
        sub.layout.clip = ClipMode::Bounds;
        copy.children.push(sub);
        b.children.push(copy);
        if active {
            b = b.class("quick-settings-active");
        }
        b
    }
    fn volume(&mut self, p: &Palette, input: bool) -> Element {
        let state = if input {
            self.state.input
        } else {
            self.state.output
        };
        let mut group = self.stack(p);
        group.layout.gap = 4.0;
        let title = if input { "Microphone" } else { "Output volume" };
        let mute_label = if state.is_some_and(|s| s.muted) {
            format!("Unmute {title}")
        } else {
            format!("Mute {title}")
        };
        let mut heading = self.row(p);
        let mut mute = self.button(
            p,
            "",
            Target::Command(QuickSettingsCommand::SetMute {
                input,
                muted: !state.is_some_and(|v| v.muted),
            }),
            state.is_some(),
        );
        mute.layout.width = Length::Px(30.0);
        mute.layout.height = Length::Px(30.0);
        let mute = self.icon_control(
            mute,
            if input {
                Icon::Microphone
            } else {
                Icon::Speaker
            },
            &mute_label,
        );
        heading.children.push(mute);
        let mut name = self.label(p, title, false);
        name.layout.height = Length::Auto;
        name.layout.padding_edges = Some(Insets::all(0.0));
        heading.children.push(name);
        let mut value = self.label(
            p,
            state.map_or("Unavailable".into(), |v| {
                format!("{}%{}", v.percent, if v.muted { " · Muted" } else { "" })
            }),
            true,
        );
        value.layout.width = Length::Px(90.0);
        value.layout.height = Length::Auto;
        value.layout.padding_edges = Some(Insets::all(0.0));
        value = value.class("quick-settings-value");
        heading.children.push(value);
        group.children.push(heading);
        group.children.push(p.slider(
            if input { INPUT } else { OUTPUT },
            0.0,
            100.0,
            state.map_or(0.0, |v| f64::from(v.percent)),
            1.0,
            state.is_some(),
        ));
        group
    }
    pub(super) fn compose(&mut self) -> Element {
        let p = Palette::new();
        self.targets.clear();
        self.next = 100;
        let gap = if self.size[0] < 360.0 { 8.0 } else { 12.0 };
        let width = (self.size[0] - 2.0 * gap).clamp(1.0, 340.0);
        let top = if self.size[1] > 160.0 { 44.0 } else { 0.0 };
        let mut root = p.panel(0).layout(Layout {
            flow: Flow::Overlay,
            height: Length::Fill,
            ..Default::default()
        });
        root = root.class("quick-settings-root");
        root.style.background = Some(ShapePaint::fill(Color::rgba(0, 0, 0, 0)));
        if self.state.high_contrast {
            root.classes.push("high-contrast".into());
        }
        let mut panel = p.surface_panel(PANEL.0).layout(Layout {
            width: Length::Px(width),
            height: Length::Px((self.size[1] - top - gap).max(1.0)),
            offset: [(self.size[0] - width - gap).max(0.0), top],
            padding: 14.0,
            gap: 10.0,
            ..Default::default()
        });
        panel = panel.class("quick-settings-panel");
        panel.component = Some(ComponentKind::Card);
        panel.style.background = None;
        let mut header = self.row(&p);
        if self.page != Page::Main {
            header.children.push(p.header_button(BACK, "‹", 30.0));
        }
        let title = match self.page {
            Page::Main => "Quick Settings",
            Page::Network => "Network",

            Page::Night => "Night Light",
            Page::Battery => "Battery charging",
        };
        let mut title = self.label(&p, title, false);
        title.layout.height = Length::Auto;
        title.layout.padding_edges = Some(Insets::all(0.0));
        title = title.class("quick-settings-title");
        header.children.push(title);
        if self.page == Page::Main {
            let mut power = p.header_button(POWER, "", 30.0);
            power.widget = Widget::Menu {
                label: String::new(),
                items: vec![
                    MenuItem::option(1, "Suspend"),
                    MenuItem::option(2, "Restart…"),
                    MenuItem::option(3, "Shut down…"),
                ],
            };
            let power = self.icon_control(power, Icon::Power, "Power menu");
            header.children.push(power);
        }
        let close = p.header_button(CLOSE, "", 30.0);
        header
            .children
            .push(self.icon_control(close, Icon::Close, "Close Quick Settings"));
        panel.children.push(header);
        let mut scroll = p.element(20, Widget::Scroll).layout(Layout {
            height: Length::Fill,
            ..Default::default()
        });
        let mut content = self.stack(&p);
        content.layout.gap = 9.0;
        match self.page {
            Page::Main => {
                let mut summary = self.row(&p);
                summary
                    .children
                    .push(if let Some(b) = self.state.battery.clone() {
                        self.battery_icon(b.percent, &b.status)
                    } else {
                        self.icon(Icon::Battery)
                    });
                let mut battery = self.stack(&p);
                battery.layout.gap = 2.0;
                if let Some(b) = self.state.battery.clone() {
                    battery.children.push(self.label(
                        &p,
                        format!("{}% · {}", b.percent, b.status),
                        false,
                    ));
                } else {
                    battery
                        .children
                        .push(self.label(&p, "No battery detected", true));
                }
                summary.children.push(battery);
                let profiles = self
                    .state
                    .power
                    .as_ref()
                    .map(|s| s.available.clone())
                    .unwrap_or_default();
                let mut profile = p.button(
                    PROFILE,
                    self.state
                        .power
                        .as_ref()
                        .map_or("Unavailable", |s| profile_name(&s.active)),
                    !profiles.is_empty(),
                );
                profile.layout.width = Length::Px(112.0);
                profile.layout.height = Length::Px(32.0);
                profile = profile.class("quick-settings-profile");
                profile.tooltip = Some("Power mode".into());
                profile.widget = Widget::Menu {
                    label: self
                        .state
                        .power
                        .as_ref()
                        .map_or("Unavailable", |s| profile_name(&s.active))
                        .into(),
                    items: profiles
                        .iter()
                        .enumerate()
                        .map(|(i, s)| MenuItem::option(i as u64 + 1, profile_name(s)))
                        .collect(),
                };
                summary.children.push(profile);
                content.children.push(summary);
                let mut tiles = self.row(&p);
                let network = self
                    .state
                    .network
                    .as_ref()
                    .map_or("Unavailable".into(), Network::summary);
                let network_active = self
                    .state
                    .network
                    .as_ref()
                    .is_some_and(|n| n.ethernet.is_some() || n.wireless.is_some());
                tiles.children.push(self.tile(
                    &p,
                    Icon::Network,
                    "Network",
                    network,
                    Target::Page(Page::Network),
                    (true, network_active),
                ));
                tiles.children.push(
                    self.tile(
                        &p,
                        Icon::Bluetooth,
                        "Bluetooth",
                        self.state
                            .bluetooth
                            .map_or("Unavailable", |v| if v { "On" } else { "Off" })
                            .into(),
                        Target::Command(QuickSettingsCommand::SetBluetooth {
                            enabled: self.state.bluetooth != Some(true),
                        }),
                        (
                            self.state.bluetooth.is_some(),
                            self.state.bluetooth == Some(true),
                        ),
                    ),
                );
                content.children.push(tiles);
                let mut audio = self.group(&p);
                audio.children.push(self.volume(&p, false));
                audio.children.push(self.separator());
                audio.children.push(self.volume(&p, true));
                audio.children.push(self.separator());
                let mut devices = self.button(
                    &p,
                    "Audio devices & apps  ↗",
                    Target::Command(QuickSettingsCommand::OpenAudio),
                    true,
                );
                devices = devices.class("quick-settings-device-link");
                devices.layout.height = Length::Px(32.0);
                audio.children.push(devices);
                content.children.push(audio);
                let mut display = self.group(&p);
                let mut brightness = self.row(&p);
                brightness.children.push(self.icon(Icon::Brightness));
                brightness
                    .children
                    .push(self.label(&p, "Brightness", false));
                let mut value = self.label(
                    &p,
                    self.state
                        .brightness
                        .map_or("Unavailable".into(), |v| format!("{v}%")),
                    true,
                );
                value.layout.width = Length::Px(80.0);
                value = value.class("quick-settings-value");
                brightness.children.push(value);
                display.children.push(brightness);
                display.children.push(p.slider(
                    BRIGHTNESS,
                    1.0,
                    100.0,
                    self.state.brightness.map_or(1.0, f64::from),
                    1.0,
                    self.state.brightness.is_some(),
                ));
                display.children.push(self.separator());
                let state = if self.state.night.active {
                    "On"
                } else if self.state.night.enabled {
                    "Scheduled"
                } else {
                    "Off"
                };
                let mut night = self.button(
                    &p,
                    format!("Night Light · {state}    ›"),
                    Target::Page(Page::Night),
                    true,
                );
                night = night.class("quick-settings-navigation");
                night.layout.height = Length::Px(34.0);
                display.children.push(night);
                content.children.push(display);
                if let Some(b) = self.state.battery.clone() {
                    let summary = b
                        .configured_limit
                        .or(b.driver_limit)
                        .map_or_else(|| "Unavailable".to_owned(), |percent| format!("{percent}%"));
                    content.children.push(self.button(
                        &p,
                        format!("Charge limit · {summary}    ›"),
                        Target::Page(Page::Battery),
                        true,
                    ));
                }
            }
            Page::Network => {
                content.children.push(self.button(
                    &p,
                    "Manage networks ↗",
                    Target::Command(QuickSettingsCommand::OpenNetwork),
                    true,
                ));
                if let Some(n) = self.state.network.clone() {
                    content.children.push(self.label(&p, n.summary(), false));
                    if n.ethernet.is_some() {
                        content.children.push(self.button(
                            &p,
                            "Disconnect Ethernet",
                            Target::Command(QuickSettingsCommand::DisconnectEthernet),
                            true,
                        ))
                    }
                    content.children.push(self.button(
                        &p,
                        if n.wifi {
                            "Wi-Fi · On"
                        } else {
                            "Wi-Fi · Off"
                        },
                        Target::Command(QuickSettingsCommand::SetWifi { enabled: !n.wifi }),
                        true,
                    ));
                    content
                        .children
                        .push(self.label(&p, "Saved connections", false));
                    for c in n.connections.into_iter().take(64) {
                        content.children.push(self.button(
                            &p,
                            format!(
                                "{} · {}",
                                c.name,
                                if c.wired { "Ethernet" } else { "Wi-Fi" }
                            ),
                            Target::Command(QuickSettingsCommand::Connect { uuid: c.uuid }),
                            true,
                        ))
                    }
                } else {
                    content
                        .children
                        .push(self.label(&p, "NetworkManager is unavailable.", true))
                }
            }
            Page::Night => {
                content.children.push(self.button(
                    &p,
                    if self.night.enabled {
                        "Night Light · On"
                    } else {
                        "Night Light · Off"
                    },
                    Target::NightToggle,
                    true,
                ));
                content.children.push(self.label(
                    &p,
                    format!("Color temperature · {} K", self.night.temperature),
                    false,
                ));
                content.children.push(p.slider(
                    TEMPERATURE,
                    2500.0,
                    6500.0,
                    f64::from(self.night.temperature),
                    100.0,
                    true,
                ));
                content.children.push(self.button(
                    &p,
                    if self.night.start.is_some() {
                        "Schedule · Custom hours"
                    } else {
                        "Schedule · Manual"
                    },
                    Target::Schedule,
                    true,
                ));
                if let (Some(start), Some(end)) = (self.night.start, self.night.end) {
                    for (id, label, v) in [(START, "From", start), (END, "Until", end)] {
                        content.children.push(self.label(
                            &p,
                            format!("{label} · {}", clock(v)),
                            false,
                        ));
                        content
                            .children
                            .push(p.slider(id, 0.0, 1439.0, f64::from(v), 15.0, true))
                    }
                }
            }
            Page::Battery => {
                if let Some(b) = self.state.battery.clone() {
                    content.children.push(self.label(
                        &p,
                        format!("Battery · {}% · {}", b.percent, b.status),
                        false,
                    ));
                    content.children.push(self.label(
                        &p,
                        format!("Selected limit · {}%", self.limit),
                        false,
                    ));
                    content.children.push(p.slider(
                        LIMIT,
                        50.0,
                        100.0,
                        f64::from(self.limit),
                        1.0,
                        b.driver_limit.is_some()
                            && !matches!(self.charge_feedback, ChargeFeedback::Applying(_)),
                    ));
                    content.children.push(self.button(
                        &p,
                        if matches!(self.charge_feedback, ChargeFeedback::Applying(_)) {
                            "Applying…"
                        } else {
                            "Apply selected limit"
                        },
                        Target::ApplyLimit,
                        b.driver_limit.is_some()
                            && !matches!(self.charge_feedback, ChargeFeedback::Applying(_)),
                    ));
                    content.children.push(self.label(
                        &p,
                        "Applying may request administrator authentication. The driver report does not confirm physical charging behavior.",
                        true,
                    ));
                    if let Some(driver_limit) = b.driver_limit {
                        content.children.push(self.label(
                            &p,
                            format!("Driver-reported threshold · {driver_limit}%"),
                            true,
                        ));
                    } else {
                        content.children.push(self.label(
                            &p,
                            "No driver-reported threshold is available for custom limits.",
                            true,
                        ));
                    }
                    match b.charge_limit.as_ref() {
                        Some(limit) if !limit.supported => content.children.push(self.label(
                            &p,
                            "UPower charge-limit preset unsupported.",
                            true,
                        )),
                        Some(limit) if !limit.enabled => {
                            content
                                .children
                                .push(self.label(&p, "UPower preset disabled.", true))
                        }
                        Some(limit) => {
                            let preset = match (limit.start_threshold, limit.end_threshold) {
                                (Some(start), Some(end)) => {
                                    format!("Start at {start}%, stop at {end}%")
                                }
                                (None, Some(end)) => format!("Stop at {end}%"),
                                (Some(start), None) => format!("Start at {start}%"),
                                (None, None) => "threshold not reported".into(),
                            };
                            content.children.push(self.label(
                                &p,
                                format!("UPower preset active · {preset}"),
                                true,
                            ));
                            content.children.push(self.label(
                                &p,
                                "Applying a custom limit disables the UPower preset.",
                                true,
                            ));
                        }
                        None => content.children.push(self.label(
                            &p,
                            "UPower charge-limit state is unavailable.",
                            true,
                        )),
                    }
                    let feedback = match &self.charge_feedback {
                        ChargeFeedback::Idle => None,
                        ChargeFeedback::Applying(percent) => Some(format!(
                            "Applying {percent}% limit… Complete authentication if requested."
                        )),
                        ChargeFeedback::Applied(percent) => {
                            Some(format!("Driver accepted the requested {percent}% limit."))
                        }
                        ChargeFeedback::Failed(error) => {
                            Some(format!("Could not update charge-limit state: {error}"))
                        }
                    };
                    if let Some(feedback) = feedback {
                        content.children.push(self.label(&p, feedback, false));
                    }
                }
            }
        }

        if let Some(error) = self.error.clone() {
            content
                .children
                .push(self.label(&p, error.chars().take(250).collect::<String>(), true))
        }
        if !self.state.errors.is_empty() {
            content.children.push(
                self.label(
                    &p,
                    self.state
                        .errors
                        .join("\n")
                        .chars()
                        .take(600)
                        .collect::<String>(),
                    true,
                ),
            )
        }
        scroll.children.push(content);
        panel.children.push(scroll);
        root.children.push(panel);
        let mut dialog = p
            .element(CONFIRMATION, Widget::Popup { modal: true })
            .layout(Layout {
                width: Length::Px((self.size[0] - 24.0).clamp(1.0, 340.0)),
                padding: 16.0,
                gap: 12.0,
                ..Default::default()
            });
        dialog.visible = self.confirmation.is_some();
        dialog = dialog.class("quick-settings-dialog");
        dialog.component = Some(ComponentKind::Card);
        dialog.style.background = None;
        if let Some(command) = self.confirmation.clone() {
            let (title, body, action) = match command {
                QuickSettingsCommand::Suspend => {
                    ("Suspend?", "Put the computer to sleep?", "Suspend")
                }
                QuickSettingsCommand::Reboot => {
                    ("Restart?", "Unsaved work may be lost.", "Restart")
                }
                _ => ("Shut down?", "Unsaved work may be lost.", "Shut down"),
            };
            dialog.children.push(self.label(&p, title, false));
            dialog.children.push(self.label(&p, body, true));
            let mut actions = self.row(&p);
            actions
                .children
                .push(p.button(CANCEL_CONFIRMATION, "Cancel", true));
            let mut confirm = p.button(CONFIRM_ACTION, action, true);
            confirm = confirm.class("quick-settings-confirm");
            actions.children.push(confirm);
            dialog.children.push(actions);
        }
        root.children.push(dialog);
        knave_shell_ui::styles::root(root, self.theme)
    }
}

#[cfg(test)]
mod icon_tests {
    use super::*;
    #[test]
    fn user_theme_icon_color_updates_and_invalid_css_is_atomic() {
        let mut app = QuickSettings::new();
        app.theme_policy_changed(Theme::Light.into(), "icon { color: #123456; }")
            .expect("valid user icon color");
        assert_eq!(app.icon_color, Color::rgba(0x12, 0x34, 0x56, 255));
        assert_eq!(app.scene.theme(), Theme::Light);

        assert!(
            app.theme_policy_changed(Theme::Dark.into(), "icon { color: invalid; }")
                .is_err()
        );
        assert_eq!(app.icon_color, Color::rgba(0x12, 0x34, 0x56, 255));
        assert_eq!(app.scene.theme(), Theme::Light);
    }

    #[test]
    fn battery_fill_and_cache_follow_real_charge_state() {
        assert_eq!(battery_fill_width(0), 0.0);
        assert_eq!(battery_fill_width(100), 11.0);
        assert_eq!(battery_fill_width(255), 11.0);
        let color = text_color(Theme::Dark.into(), "").expect("default theme is valid");
        let empty = battery_image(0, "Not charging", color);
        let full = battery_image(100, "Not charging", color);
        let charging = battery_image(100, "Charging", color);
        let filled_columns = |image: &UiImage| {
            (6..17)
                .filter(|x| image.pixels()[(12 * 24 + x) * 4 + 3] > 0)
                .count()
        };
        assert_eq!(
            empty.cache_key(),
            battery_image(0, "Not charging", color).cache_key()
        );
        assert_eq!(filled_columns(&empty), 0);
        assert_eq!(filled_columns(&full), 11);
        assert_ne!(empty.pixels(), full.pixels());
        assert_ne!(full.pixels(), charging.pixels());
    }

    #[test]
    fn monochrome_policy_tints_shell_icons_neutrally() {
        let policy = ThemePolicy::new(Theme::Dark, Contrast::High, Chroma::Monochrome);
        let color = text_color(policy, "").expect("bundled monochrome policy is valid");
        assert_eq!(color.red, color.green);
        assert_eq!(color.green, color.blue);
    }
}
