use super::*;
use knave_ui::toolkit::{ButtonVariant, ComponentKind};
use knave_ui::{ShapePaint, TextWrap};

fn fallback_icon() -> Option<knave_ui::UiImage> {
    knave_shell_ui::status_icons::bell()
}

pub(super) fn card_id(id: u64) -> u64 {
    1000 + id * 32
}
struct Palette;
impl Palette {
    fn new() -> Self {
        Self
    }
    fn style(&self, e: &mut Element) {
        e.style.text.wrap = TextWrap::WordOrGlyph;
    }
    fn panel(&self, id: u64) -> Element {
        let mut e = Element::new(id, Widget::Panel)
            .without_surface()
            .class("shell-structure")
            .class("notifications-structure");
        self.style(&mut e);
        e.style.background = Some(ShapePaint::fill(Color::rgba(0, 0, 0, 0)));
        e
    }
    fn surface_panel(&self, id: u64) -> Element {
        let mut e = Element::new(id, Widget::Panel);
        self.style(&mut e);
        e
    }
    fn text(&self, id: u64, value: String, muted: bool) -> Element {
        let mut e = if muted {
            Element::subtitle(id, value)
        } else {
            Element::new(id, Widget::Text(value))
        };
        e = e.class("shell-text").class("notifications-text");
        if muted {
            e = e.class("shell-muted").class("notifications-muted");
        }
        self.style(&mut e);
        e
    }
    fn close_button(&self, id: u64, keyboard_focus: bool) -> Element {
        let mut e = Element::new(id, Widget::Button("×".into())).layout(Layout {
            width: Length::Px(32.0),
            height: Length::Px(32.0),
            ..Default::default()
        });
        e = e
            .class("shell-control")
            .class("notifications-button")
            .class("notifications-close");
        self.style(&mut e);
        e.button_variant = Some(ButtonVariant::Ghost);
        if keyboard_focus {
            e.classes.push("keyboard-focus-visible".into());
            e.classes.push("notifications-keyboard-focus".into());
        }
        e
    }
    fn page_button(&self, id: u64, value: &str) -> Element {
        let mut e = self.button(id, value);
        e.layout.width = Length::Px(36.0);
        e.layout.height = Length::Px(36.0);
        e.layout.padding = 0.0;
        e = e.class("notifications-page-button");
        e
    }
    fn button(&self, id: u64, value: &str) -> Element {
        let mut e = Element::new(id, Widget::Button(value.into())).layout(Layout {
            padding: 8.0,
            ..Default::default()
        });
        e = e.class("shell-control").class("notifications-button");
        self.style(&mut e);
        e
    }
    fn text_button(&self, id: u64, value: &str) -> Element {
        let mut e = Element::new(id, Widget::Button(value.into())).layout(Layout {
            width: Length::Auto,
            height: Length::Px(26.0),
            padding: 4.0,
            ..Default::default()
        });
        e = e.class("shell-control").class("notifications-text-button");
        self.style(&mut e);
        e.button_variant = Some(ButtonVariant::Ghost);
        e
    }
}
fn short(s: &str, max: usize) -> String {
    let mut result: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        result.push('…');
    }
    result
}
fn panel_geometry(width: f32) -> (f32, f32) {
    let gap = if width < 360.0 { 6.0 } else { 12.0 };
    (gap, (width - gap * 2.0).clamp(1.0, 340.0))
}
impl Notifications {
    pub(super) fn compose(&mut self) -> Element {
        let p = Palette::new();
        self.targets.clear();
        let width = self.size[0].max(1.0);
        let height = self.size[1].max(1.0);
        let (gap, panel_width) = panel_geometry(width);
        let top = if height > 160.0 { 44.0 } else { 0.0 };
        let mut root = p.panel(0);
        root.layout.flow = Flow::Overlay;
        root.layout.height = Length::Fill;
        root = root.class("notifications-root");
        root.style.background = Some(ShapePaint::fill(Color::rgba(0, 0, 0, 0)));
        if self.state.high_contrast {
            root.classes.push("high-contrast".into());
        }
        let mut panel = p.surface_panel(PANEL.0).layout(Layout {
            width: Length::Px(panel_width),
            height: if self.state.center_open {
                Length::Px((height - top - gap).max(1.0))
            } else {
                Length::Auto
            },
            offset: [(width - panel_width - gap).max(0.0), top],
            padding: 12.0,
            gap: 8.0,
            ..Default::default()
        });
        if self.state.center_open {
            panel = panel.class("notifications-panel");
            panel.component = Some(ComponentKind::Card);
            panel.style.background = None;
            let mut heading = p.panel(10).layout(Layout {
                flow: Flow::Row,
                align: Align::Center,
                height: Length::Auto,
                gap: 8.0,
                ..Default::default()
            });
            let mut title = p.text(11, "Notifications".into(), false);
            title.layout.height = Length::Auto;
            title = title.class("notifications-title");
            heading.children.push(title);
            let mut close = p.close_button(CLOSE, self.keyboard_focus_visible);
            close.tooltip = Some("Close notification center".into());
            heading.children.push(close);
            panel.children.push(heading);
            let mut dnd = p.surface_panel(20).layout(Layout {
                flow: Flow::Row,
                align: Align::Center,
                gap: 10.0,
                padding: 12.0,
                ..Default::default()
            });
            dnd = dnd.class("notifications-dnd");
            dnd.component = Some(ComponentKind::Card);
            dnd.style.background = None;
            let mut dnd_copy = p.panel(21).layout(Layout {
                height: Length::Auto,
                ..Default::default()
            });
            let mut dnd_title = p.text(22, "Do not disturb".into(), false);
            dnd_title = dnd_title.class("notifications-dnd-title");
            dnd_copy.children.push(dnd_title);
            let mut dnd_subtitle = p.text(
                23,
                if self.state.dnd {
                    "Banners paused · history stays on".into()
                } else {
                    "Silence banners, keep history".into()
                },
                true,
            );
            dnd_subtitle = dnd_subtitle.class("notifications-dnd-subtitle");
            dnd_copy.children.push(dnd_subtitle);
            dnd.children.push(dnd_copy);
            let mut toggle = Element::new(
                DND,
                Widget::Toggle {
                    label: String::new(),
                    checked: self.state.dnd,
                },
            )
            .layout(Layout {
                width: Length::Px(40.0),
                height: Length::Px(24.0),
                ..Default::default()
            });
            toggle = toggle.class("shell-control").class("notifications-toggle");
            p.style(&mut toggle);
            toggle.style.background = Some(ShapePaint::fill(Color::rgba(0, 0, 0, 0)));
            toggle.enabled = self.state.owned;
            toggle.tooltip = Some("Hide popups from all apps. Notifications remain in history; turning this off does not replay them.".into());
            dnd.children.push(toggle);
            panel.children.push(dnd);
            let mut controls = p.panel(12).layout(Layout {
                flow: if panel_width < 290.0 {
                    Flow::Column
                } else {
                    Flow::Row
                },
                align: if panel_width < 290.0 {
                    Align::Start
                } else {
                    Align::Center
                },
                gap: 8.0,
                ..Default::default()
            });
            let mut recent = p.text(
                24,
                format!("RECENT · {}", self.state.history().count()),
                true,
            );
            recent.layout.height = Length::Auto;
            recent = recent.class("notifications-recent");
            controls.children.push(recent);
            let mut read = p.text_button(READ, "Mark all read");
            read.layout.width = Length::Px(84.0);
            read.enabled = self.state.history().next().is_some();
            controls.children.push(read);
            let mut clear = p.text_button(CLEAR, "Clear all");
            clear.layout.width = Length::Px(64.0);
            clear.enabled = self.state.history().next().is_some();
            controls.children.push(clear);
            panel.children.push(controls);
            if !self.state.owned {
                panel.children.push(p.text(13,
                    "An external service delivers new notifications; saved Knave history is below.".into(),
                    true,
                ));
            }
            if let Some(error) = &self.error {
                panel.children.push(p.text(14, short(error, 180), true));
            }
            if let Some(error) = &self.state.error {
                panel.children.push(p.text(15, short(error, 180), true));
            }
            let mut scroll = Element::new(SCROLL, Widget::Scroll).layout(Layout {
                height: Length::Fill,
                ..Default::default()
            });
            let compact_controls = if height < 480.0 {
                panel.children.split_off(1)
            } else {
                Vec::new()
            };
            let mut entries = p.panel(16).layout(Layout {
                gap: 9.0,
                ..Default::default()
            });
            entries.children.extend(compact_controls);
            let notifications: Vec<_> = self
                .state
                .history()
                .skip(self.page * PAGE)
                .take(PAGE)
                .cloned()
                .collect();
            if notifications.is_empty() {
                entries
                    .children
                    .push(p.text(17, "You're all caught up".into(), true));
            }
            let mut groups: Vec<(String, Vec<Notification>)> = Vec::new();
            for n in notifications {
                if let Some((_, entries)) = groups.iter_mut().find(|(app, _)| *app == n.app_id) {
                    entries.push(n)
                } else {
                    groups.push((n.app_id.clone(), vec![n]))
                }
            }
            for (_, group) in groups {
                for n in group {
                    entries.children.push(self.card(&n, &p, false));
                }
            }
            scroll.children.push(entries);
            panel.children.push(scroll);
            let mut footer = p.panel(18).layout(Layout {
                flow: Flow::Row,
                align: Align::Center,
                gap: 6.0,
                ..Default::default()
            });
            let mut previous = p.page_button(PREVIOUS, "‹");
            previous.enabled = self.page > 0;
            previous.tooltip = Some("Previous notifications".into());
            footer.children.push(previous);
            let pages = self.state.history().count().div_ceil(PAGE).max(1);
            let mut page_label = p.text(
                19,
                format!(
                    "{} / {} · {} days",
                    self.page + 1,
                    pages,
                    self.state.history_days
                ),
                true,
            );
            page_label.layout.height = Length::Auto;
            page_label = page_label.class("notifications-page-label");
            footer.children.push(page_label);
            let mut next = p.page_button(NEXT, "›");
            next.enabled = self.page + 1 < pages;
            next.tooltip = Some("Older notifications".into());
            footer.children.push(next);
            panel.children.push(footer);
        } else {
            panel = panel.without_surface().class("notifications-structure");
            panel.style.background = Some(ShapePaint::fill(Color::rgba(0, 0, 0, 0)));
            panel.layout.padding = 0.0;
            panel.layout.height = Length::Px((height - top - gap).max(0.0));
            let notifications: Vec<_> = self
                .state
                .notifications
                .iter()
                .filter(|n| n.banner)
                .take(3)
                .cloned()
                .collect();
            let mut scroll = Element::new(SCROLL, Widget::Scroll).layout(Layout {
                height: Length::Fill,
                ..Default::default()
            });
            let mut stack = p.panel(16);
            for n in notifications {
                stack.children.push(self.card(&n, &p, true));
            }
            scroll.children.push(stack);
            panel.children.push(scroll);
        }
        root.children.push(panel);
        knave_shell_ui::styles::root(root, self.theme)
    }
    fn card(&mut self, n: &Notification, p: &Palette, banner: bool) -> Element {
        let id = card_id(n.id);
        let mut card = p
            .surface_panel(id)
            .component(ComponentKind::Card)
            .layout(Layout {
                padding: 12.0,
                gap: 7.0,
                ..Default::default()
            });
        card.style.background = None;
        card = card.class("notifications-card");
        if n.unread {
            card = card.class("notifications-unread");
        }
        if n.urgency == 2 {
            card = card.class("notifications-critical");
        }
        let mut head = p.panel(id + 1).layout(Layout {
            flow: Flow::Row,
            align: Align::Center,
            gap: 8.0,
            ..Default::default()
        });
        let icon = if !n.icon.is_empty() {
            n.icon.as_str()
        } else if !n.app_id.is_empty() {
            n.app_id.strip_suffix(".desktop").unwrap_or(&n.app_id)
        } else if !n.app.is_empty() {
            n.app.as_str()
        } else {
            crate::overview::icons::FALLBACK
        };
        if self.icons.needs_request(icon)
            && let Some(loader) = &self.loader
            && loader.submit(Job::Icon(icon.into()))
        {
            self.icons.requested(icon);
        }
        if let Some(image) = self.icons.get(icon).cloned().or_else(fallback_icon) {
            head.children.push(
                Element::new(id + 2, Widget::Image(image, Default::default())).layout(Layout {
                    width: Length::Px(24.0),
                    height: Length::Px(24.0),
                    ..Default::default()
                }),
            );
        }
        let age = crate::notifications::age(n.created_ms);
        let mut metadata = p.text(
            id + 3,
            format!(
                "{} · {}{}",
                if n.app.is_empty() {
                    "Application"
                } else {
                    &n.app
                },
                age,
                if n.unread { " · Unread" } else { "" }
            ),
            true,
        );
        metadata = metadata.class("notifications-metadata");
        head.children.push(metadata);
        let mut dismiss = p.close_button(id + 4, self.keyboard_focus_visible);
        dismiss.tooltip = Some("Dismiss notification".into());
        self.targets.insert(dismiss.id, Target::Dismiss(n.id));
        head.children.push(dismiss);
        card.children.push(head);
        let summary = short(&n.summary, if banner { 150 } else { 512 });
        let mut title = p.text(id + 5, summary, false);
        title = title.class("notifications-summary");
        card.children.push(title);
        let expanded = self.expanded.contains(&n.id);
        let body = short(
            &n.body,
            if expanded {
                4096
            } else if banner {
                180
            } else {
                360
            },
        );
        if !body.is_empty() {
            let mut body = p.text(id + 6, body, false);
            body = body.class("notifications-body");
            card.children.push(body);
        }
        if let Some(source) = &n.image {
            if !self.previews.contains_key(&n.id)
                && let Some(image) = knave_ui::UiImage::from_rgba(
                    source.width(),
                    source.height(),
                    source.pixels().to_vec(),
                )
            {
                self.previews.insert(n.id, (source.clone(), image));
            }
            if let Some((_, image)) = self.previews.get(&n.id) {
                let (_, panel_width) = panel_geometry(self.size[0]);
                let available = (panel_width - if banner { 24.0 } else { 48.0 }).max(1.0);
                let height = (available * source.height() as f32 / source.width() as f32)
                    .min(if banner { 180.0 } else { 220.0 });
                card.children.push(
                    Element::new(
                        id + 11,
                        Widget::Image(
                            image.clone(),
                            knave_ui::ImageStyle {
                                fit: knave_ui::ImageFit::Contain,
                                ..Default::default()
                            },
                        ),
                    )
                    .layout(Layout {
                        height: Length::Px(height),
                        ..Default::default()
                    }),
                );
            }
        }
        if !banner && n.body.chars().count() > 360 {
            let more = p.button(id + 7, if expanded { "Show less" } else { "Show more" });
            self.targets.insert(more.id, Target::Expand(n.id));
            card.children.push(more);
        }
        if let Some(value) = n.progress {
            card.children
                .push(p.text(id + 8, format!("Progress: {value}%"), true));
            card.children.push(
                Element::new(
                    id + 9,
                    Widget::Progress {
                        value: Some(f64::from(value) / 100.0),
                    },
                )
                .layout(Layout {
                    height: Length::Px(4.0),
                    ..Default::default()
                })
                .class("notifications-progress"),
            );
        }
        if n.live {
            for (index, action) in n
                .actions
                .iter()
                .filter(|action| action.key != "default")
                .take(MAX_ACTIONS)
                .enumerate()
            {
                let mut button = p.button(id + 12 + index as u64, &short(&action.label, 100));
                button.tooltip = Some(action.label.clone());
                self.targets
                    .insert(button.id, Target::Invoke(n.id, action.key.clone()));
                button.layout.padding = 8.0;
                card.children.push(button);
            }
        }
        if !banner && !n.app_id.is_empty() && self.state.owned {
            let enabled = *self.state.app_banners.get(&n.app_id).unwrap_or(&true);
            let mut button = p.button(
                id + 21,
                if enabled {
                    "Hide this app’s popups"
                } else {
                    "Show this app’s popups"
                },
            );
            button.tooltip = Some("Controls popups from this app only. Notifications remain in history and the unread count; missed popups are not replayed.".into());
            self.targets
                .insert(button.id, Target::AppBanners(n.app_id.clone(), !enabled));
            card.children.push(button);
        }
        card
    }
}
