use super::*;
use knave_ui::{DisplayCommand, TextSpan, TextStyle};
struct Metrics;
impl TextMeasurer for Metrics {
    fn measure(&mut self, text: &str, style: &TextStyle, width: f32) -> TextLayout {
        let glyph = style.font_size * 0.6;
        let columns = (width / glyph).floor().max(1.0) as usize;
        let lines: usize = text
            .split('\n')
            .map(|s| s.chars().count().max(1).div_ceil(columns))
            .sum();
        TextLayout {
            size: [
                (text.chars().count() as f32 * glyph).min(width),
                lines as f32 * style.line_height,
            ],
            carets: Vec::new(),
        }
    }
    fn measure_spans(&mut self, spans: &[TextSpan], style: &TextStyle, width: f32) -> TextLayout {
        self.measure(
            &spans.iter().map(|s| s.text.as_str()).collect::<String>(),
            style,
            width,
        )
    }
}
fn notification(id: u64) -> Notification {
    Notification {
        id,
        app: "Example application with a long name".into(),
        app_id: "org.example.App".into(),
        summary: "A notification title that wraps safely".into(),
        body: "A message with Unicode: 日本語 العربية café. ".repeat(100),
        unread: true,
        urgency: 1,
        live: true,
        actions: (0..8)
            .map(|i| NotificationAction {
                key: format!("action{i}"),
                label: "A very long action label that must wrap without covering other buttons"
                    .repeat(3),
            })
            .collect(),
        progress: Some(100),
        ..Default::default()
    }
}
fn app(count: u64) -> Notifications {
    let mut app = Notifications::new();
    let state = NotificationSnapshot {
        owned: true,
        center_open: true,
        history_days: 7,
        history_limit: 500,
        notifications: (1..=count).rev().map(notification).collect(),
        ..Default::default()
    };
    app.apply(state);
    app
}
fn click(app: &mut Notifications, id: ElementId) {
    assert!(app.error.is_none(), "{:?}", app.error);
    let r = app.scene.bounds(id).unwrap();
    let point = [r.x + r.width / 2.0, r.y + r.height / 2.0];
    app.input(Input::PointerMove(point));
    app.input(Input::PointerDown(point));
    app.frame(app.size, &mut Metrics);
    app.input(Input::PointerUp(point));
}
#[test]
fn narrow_short_and_large_views_keep_panel_and_navigation_in_bounds() {
    for size in [
        [320.0, 240.0],
        [320.0, 568.0],
        [400.0, 800.0],
        [1280.0, 800.0],
        [1920.0, 1080.0],
    ] {
        for light in [false, true] {
            for contrast in [false, true] {
                let mut app = app(500);
                app.theme_policy_changed(
                    (if light { Theme::Light } else { Theme::Dark }).into(),
                    "",
                )
                .expect("notification theme should apply");
                app.state.light = light;
                app.state.high_contrast = contrast;
                app.apply(app.state.clone());
                let list = app.frame(size, &mut Metrics).clone();
                assert!(app.error.is_none(), "{:?}", app.error);
                assert_eq!(list.clear_color.alpha, 0);
                assert_eq!(app.theme, if light { Theme::Light } else { Theme::Dark });
                assert_eq!(
                    app.scene
                        .computed_style(ElementId(0))
                        .unwrap()
                        .style
                        .background
                        .and_then(|paint| paint.fill)
                        .unwrap()
                        .alpha,
                    0,
                    "overlay root must stay transparent at {size:?}"
                );
                let panel = app.scene.bounds(PANEL).unwrap();
                assert!(panel.x >= 0.0 && panel.y >= 0.0);
                assert!(panel.x + panel.width <= size[0]);
                assert!(panel.y + panel.height <= size[1]);
                for id in [CLOSE, PREVIOUS, NEXT] {
                    let r = app.scene.bounds(ElementId(id)).unwrap();
                    assert!(r.x >= panel.x && r.y >= panel.y);
                    assert!(
                        r.x + r.width <= panel.x + panel.width + 0.1,
                        "{size:?} {id} {r:?}"
                    );
                    assert!(
                        r.y + r.height <= panel.y + panel.height + 0.1,
                        "{size:?} {id} {r:?}"
                    );
                }
                for (id, label) in [(PREVIOUS, "‹"), (NEXT, "›")] {
                    let bounds = app.scene.bounds(ElementId(id)).unwrap();
                    assert_eq!([bounds.width, bounds.height], [36.0, 36.0]);
                    let (text_bounds, style) = list
                        .commands
                        .iter()
                        .find_map(|command| match command {
                            DisplayCommand::Text {
                                bounds,
                                text,
                                style,
                                ..
                            } if text == label => Some((bounds, style)),
                            _ => None,
                        })
                        .unwrap();
                    assert_eq!(style.align, knave_ui::TextAlign::Center);
                    assert!(
                        (text_bounds.x + text_bounds.width / 2.0 - bounds.x - bounds.width / 2.0)
                            .abs()
                            < 0.1
                    );
                    assert!(
                        (text_bounds.y + text_bounds.height / 2.0 - bounds.y - bounds.height / 2.0)
                            .abs()
                            < 0.1
                    );
                }
                for command in &list.commands {
                    if let DisplayCommand::Shape { paint, .. } = command {
                        let radius = paint.radii.top_left;
                        assert!(radius.is_finite() && (0.0..=32.0).contains(&radius));
                        assert!(paint.radii.top_right.is_finite());
                        assert!(paint.radii.bottom_right.is_finite());
                        assert!(paint.radii.bottom_left.is_finite());
                    }
                }
                let scroll = app.scene.bounds(ElementId(SCROLL)).unwrap();
                let footer = app.scene.bounds(ElementId(NEXT)).unwrap();
                assert!(scroll.y + scroll.height <= footer.y + 0.1);
                assert!(app.error.is_none(), "{:?}", app.error);
                for c in &list.commands {
                    let (r, clip) = match c {
                        DisplayCommand::Shape { bounds, clip, .. }
                        | DisplayCommand::Text { bounds, clip, .. }
                        | DisplayCommand::RichText { bounds, clip, .. }
                        | DisplayCommand::Image { bounds, clip, .. } => (*bounds, *clip),
                        DisplayCommand::BeginOpacity { .. } | DisplayCommand::EndOpacity => {
                            continue;
                        }
                    };
                    assert!(
                        r.x.is_finite() && r.y.is_finite() && r.width >= 0.0 && r.height >= 0.0
                    );
                    if let Some(clip) = clip {
                        assert!(clip.x >= 0.0 && clip.y >= 0.0);
                        assert!(clip.x + clip.width <= size[0] + 0.1);
                        assert!(clip.y + clip.height <= size[1] + 0.1);
                    }
                }
            }
        }
    }
}

#[test]
fn notification_panel_uses_proposed_surface_and_a_real_dnd_toggle() {
    let mut app = app(2);
    app.state.dnd = true;
    let list = app.frame([400.0, 800.0], &mut Metrics).clone();
    let panel = app.scene.bounds(PANEL).unwrap();
    let toggle_bounds = app.scene.bounds(ElementId(DND)).unwrap();
    let dnd_bounds = app.scene.bounds(ElementId(20)).unwrap();
    assert!(toggle_bounds.x >= dnd_bounds.x);
    assert!(toggle_bounds.x + toggle_bounds.width <= dnd_bounds.x + dnd_bounds.width);
    assert!(toggle_bounds.y >= dnd_bounds.y);
    assert!(toggle_bounds.y + toggle_bounds.height <= dnd_bounds.y + dnd_bounds.height);
    assert_eq!(
        app.scene.widget(ElementId(DND)),
        Some(&Widget::Toggle {
            label: String::new(),
            checked: true,
        })
    );
    assert!(app.scene.bounds(ElementId(READ)).unwrap().width > 0.0);
    assert!(app.scene.bounds(ElementId(CLEAR)).unwrap().width > 0.0);
    for id in [PANEL.0, 20, view::card_id(1)] {
        let paint = app
            .scene
            .computed_style(ElementId(id))
            .unwrap()
            .style
            .background
            .expect("semantic panel component should receive a theme surface");
        assert!(paint.fill.is_some_and(|fill| fill.alpha > 0));
    }
    assert!(list.commands.iter().any(|command| matches!(
        command,
        DisplayCommand::Text { text, style, .. }
            if text == "Notifications" && style.font_size == TextStyle::default().font_size
    )));
    assert!(list.commands.iter().any(|command| matches!(
        command,
        DisplayCommand::Text { text, .. } if text == "RECENT · 2"
    )));
    assert_eq!(panel.width, 340.0);
}

#[test]
fn dnd_toggle_retains_pointer_activation() {
    let mut app = app(1);
    app.state.dnd = false;
    app.frame([400.0, 800.0], &mut Metrics);
    click(&mut app, ElementId(DND));
    assert_eq!(
        app.scene.widget(ElementId(DND)),
        Some(&Widget::Toggle {
            label: String::new(),
            checked: true,
        })
    );
}

#[test]
fn external_notification_service_cannot_toggle_global_dnd() {
    let mut app = app(1);
    app.state.owned = false;
    app.frame([400.0, 800.0], &mut Metrics);
    assert_eq!(
        app.scene.widget(ElementId(DND)),
        Some(&Widget::Toggle {
            label: String::new(),
            checked: false,
        })
    );
    click(&mut app, ElementId(DND));
    assert_eq!(
        app.scene.widget(ElementId(DND)),
        Some(&Widget::Toggle {
            label: String::new(),
            checked: false,
        })
    );
}
#[test]
fn paging_reaches_all_history_without_exceeding_scene_budget() {
    let mut app = app(500);
    app.frame([400.0, 800.0], &mut Metrics);
    assert!(app.scene.bounds(ElementId(view::card_id(500))).is_some());
    assert!(app.scene.bounds(ElementId(view::card_id(480))).is_none());
    for _ in 0..24 {
        click(&mut app, ElementId(NEXT));
        app.frame(app.size, &mut Metrics);
    }
    assert_eq!(app.page, 24);
    assert!(app.scene.bounds(ElementId(view::card_id(1))).is_some());
    assert!(app.scene.bounds(ElementId(view::card_id(500))).is_none());
}
#[test]
fn banner_input_regions_exclude_desktop_and_never_request_keyboard() {
    let mut app = app(3);
    app.state.center_open = false;
    for n in &mut app.state.notifications {
        n.banner = true;
    }
    app.frame([1280.0, 800.0], &mut Metrics);
    assert_eq!(app.keyboard_mode(), Some(KeyboardMode::None));
    for r in app.input_region().unwrap() {
        assert!(r.x >= 850.0);
        assert!(r.y >= 44.0);
        assert!(r.y + r.height <= 800.0);
    }
    assert!(
        !app.input_region()
            .unwrap()
            .iter()
            .any(|r| contains(*r, [100.0, 100.0]))
    );
    assert!(app.surface_visible());
}
#[test]
fn empty_external_and_disconnected_states_are_honest() {
    let mut app = app(0);
    app.state.owned = false;
    app.frame([320.0, 568.0], &mut Metrics);
    assert!(!app.scene.widget(ElementId(DND)).is_none());
    assert!(
        app.display
            .commands
            .iter()
            .any(|c| matches!(c,DisplayCommand::Text{text,..}if text.contains("external service")))
    );
    app.connected = false;
    assert!(!app.surface_visible());
}
#[test]
fn replacement_preserves_pointer_press_and_does_not_activate_a_neighbor() {
    let mut app = app(1);
    app.frame([400.0, 800.0], &mut Metrics);
    let dismiss = ElementId(view::card_id(1) + 4);
    let r = app.scene.bounds(dismiss).unwrap();
    let point = [r.x + 10.0, r.y + 10.0];
    app.input(Input::PointerDown(point));
    app.state.notifications[0].summary = "Updated while held".into();
    app.dirty = true;
    app.frame(app.size, &mut Metrics);
    assert_eq!(app.scene.capture(), Some(dismiss));
    app.input(Input::PointerUp(point));
    assert_eq!(app.scene.capture(), None);
}
#[test]
fn transient_is_not_in_center_or_badge() {
    let mut app = app(1);
    app.state.notifications[0].transient = true;
    app.frame([400.0, 800.0], &mut Metrics);
    assert_eq!(app.state.unread(), 0);
    assert!(app.scene.bounds(ElementId(view::card_id(1))).is_none());
}

#[test]
fn reopening_hidden_center_resets_keyboard_focus_to_close() {
    let mut app = app(2);
    app.frame([400.0, 800.0], &mut Metrics);
    app.scene.set_focus(Some(ElementId(DND))).unwrap();
    let mut state = app.state.clone();
    state.center_open = false;
    app.apply(state.clone());
    state.center_open = true;
    app.apply(state);
    app.frame(app.size, &mut Metrics);
    assert_eq!(app.scene.focus(), Some(ElementId(CLOSE)));
}

#[test]
fn progress_widget_tracks_card_content_width_and_accessible_percentage() {
    for width in [320.0, 400.0, 1280.0] {
        for banner in [false, true] {
            for value in [0, 1, 25, 50, 99, 100] {
                let mut app = app(1);
                app.state.center_open = !banner;
                app.state.notifications[0].banner = banner;
                app.state.notifications[0].progress = Some(value);
                app.frame([width, 800.0], &mut Metrics);
                let id = view::card_id(1);
                let bounds = app.scene.bounds(ElementId(id + 9)).unwrap();
                assert!(bounds.width > 0.0 && bounds.width <= 340.0);
                assert_eq!(
                    app.scene.widget(ElementId(id + 9)),
                    Some(&Widget::Progress {
                        value: Some(f64::from(value) / 100.0)
                    })
                );
            }
        }
    }
}

#[test]
fn close_buttons_are_compact_and_focus_follows_input_method() {
    let mut app = app(1);
    app.frame([400.0, 800.0], &mut Metrics);
    for id in [CLOSE, view::card_id(1) + 4] {
        let bounds = app.scene.bounds(ElementId(id)).unwrap();
        assert_eq!([bounds.width, bounds.height], [32.0, 32.0]);
    }
    assert!(!app.keyboard_focus_visible);
    app.input(Input::Key {
        key: Key::Tab,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    });
    app.frame(app.size, &mut Metrics);
    assert!(app.keyboard_focus_visible);
    let bounds = app.scene.bounds(ElementId(CLOSE)).unwrap();
    app.input(Input::PointerDown([bounds.x + 16.0, bounds.y + 16.0]));
    app.frame(app.size, &mut Metrics);
    assert!(!app.keyboard_focus_visible);
    assert_eq!(app.scene.capture(), Some(ElementId(CLOSE)));
}

#[test]
fn missing_notification_icon_uses_app_identity_without_recoloring() {
    let colors = vec![88, 101, 242, 255, 255, 255, 255, 255];
    for (icon, app_id, name, expected) in [
        ("", "discord", "discord", "discord"),
        ("", "discord.desktop", "discord", "discord"),
        ("", "", "discord", "discord"),
        ("sender-icon", "discord", "discord", "sender-icon"),
        ("", "", "", crate::overview::icons::FALLBACK),
    ] {
        for light in [false, true] {
            let mut app = app(1);
            app.theme_policy_changed((if light { Theme::Light } else { Theme::Dark }).into(), "")
                .expect("notification theme should apply");
            app.state.light = light;
            app.state.notifications[0].icon = icon.into();
            app.state.notifications[0].app_id = app_id.into();
            app.state.notifications[0].app = name.into();
            app.icons.finish(
                expected.into(),
                knave_ui::UiImage::from_rgba(2, 1, colors.clone()),
            );
            let list = app.frame([400.0, 800.0], &mut Metrics);
            let image = list
                .commands
                .iter()
                .find_map(|command| match command {
                    DisplayCommand::Image { image, .. } => Some(image),
                    _ => None,
                })
                .expect("notification uses its app icon");
            assert_eq!(image.pixels(), colors);
        }
    }
}

#[test]
fn unavailable_app_icon_uses_bundled_bell_without_a_loader_result() {
    let mut app = app(1);
    app.icons.finish("org.example.App".into(), None);
    let list = app.frame([400.0, 800.0], &mut Metrics);
    let image = list
        .commands
        .iter()
        .find_map(|command| match command {
            DisplayCommand::Image { image, .. } => Some(image),
            _ => None,
        })
        .expect("notification cards always have a fallback glyph");
    assert_eq!((image.width(), image.height()), (48, 48));
}

#[test]
fn image_previews_fit_cards_preserve_pixels_and_release_on_replace_or_dismiss() {
    for size in [[320.0, 240.0], [320.0, 568.0], [1280.0, 800.0]] {
        for banner in [false, true] {
            for light in [false, true] {
                for (width, height) in [(320, 160), (80, 320), (320, 1), (1, 320)] {
                    let mut app = app(1);
                    app.theme_policy_changed(
                        (if light { Theme::Light } else { Theme::Dark }).into(),
                        "",
                    )
                    .expect("notification theme should apply");
                    app.state.light = light;
                    app.state.center_open = !banner;
                    app.state.notifications[0].banner = banner;
                    let pixels = [88, 101, 242, 128].repeat((width * height) as usize);
                    app.state.notifications[0].image =
                        NotificationImage::new(width, height, pixels.clone());
                    let list = app.frame(size, &mut Metrics).clone();
                    let id = view::card_id(1) + 11;
                    let bounds = app.scene.bounds(ElementId(id)).unwrap();
                    assert!(bounds.x >= 0.0 && bounds.x + bounds.width <= size[0]);
                    assert!(bounds.height <= if banner { 180.0 } else { 220.0 });
                    let Widget::Image(image, style) = app.scene.widget(ElementId(id)).unwrap()
                    else {
                        panic!("preview widget missing")
                    };
                    for command in &list.commands {
                        if let DisplayCommand::Image {
                            clip: Some(clip), ..
                        } = command
                        {
                            assert!(clip.x >= 0.0 && clip.y >= 0.0);
                            assert!(clip.x + clip.width <= size[0]);
                            assert!(clip.y + clip.height <= size[1]);
                        }
                    }
                    assert_eq!(image.pixels(), pixels);
                    assert_eq!(style.fit, knave_ui::ImageFit::Contain);
                    assert_eq!(style.corner_radius, 0.0);
                    assert_eq!(app.previews.len(), 1);
                    let previous = image.cache_key();
                    let mut state = app.state.clone();
                    state.notifications[0].summary = "Updated text".into();
                    app.apply(state.clone());
                    app.frame(size, &mut Metrics);
                    assert_eq!(app.previews[&1].1.cache_key(), previous);
                    state.notifications[0].image = NotificationImage::new(1, 1, vec![255; 4]);
                    app.apply(state.clone());
                    app.frame(size, &mut Metrics);
                    assert_eq!(app.previews[&1].1.pixels(), [255; 4]);
                    state.notifications[0].image = None;
                    app.apply(state);
                    app.frame(size, &mut Metrics);
                    assert!(app.previews.is_empty());
                    assert!(app.scene.bounds(ElementId(id)).is_none());
                }
            }
        }
    }
    let mut app = app(1);
    app.state.notifications[0].image = NotificationImage::new(1, 1, vec![255; 4]);
    app.frame([400.0, 800.0], &mut Metrics);
    let mut state = app.state.clone();
    state.notifications.clear();
    app.apply(state);
    app.frame([400.0, 800.0], &mut Metrics);
    assert!(app.previews.is_empty());
}

#[test]
fn shell_panels_do_not_add_decorative_outlines() {
    let mut app = app(1);
    app.frame([1280.0, 800.0], &mut Metrics);
    for id in [PANEL, ElementId(CLOSE)] {
        let computed = app.scene.computed_style(id).unwrap();
        assert!(
            computed.outline.is_none_or(|outline| outline.width == 0.0),
            "idle shell element {id:?} must use the toolkit's borderless presentation"
        );
    }
}

#[test]
fn full_named_themes_cannot_paint_the_fullscreen_positioning_host() {
    for theme in [Theme::Dark, Theme::Light] {
        let mut app = app(1);
        app.theme_policy_changed(theme.into(), theme.stylesheet())
            .unwrap();
        let list = app.frame([1280.0, 800.0], &mut Metrics);
        assert!(
            !list.commands.iter().any(|command| matches!(command,
                knave_ui::DisplayCommand::Shape { bounds, paint, .. }
                    if bounds.x <= 400.0 && bounds.y <= 400.0
                        && bounds.x + bounds.width > 400.0 && bounds.y + bounds.height > 400.0
                        && paint.fill.is_some_and(|color| color.alpha > 0)
            )),
            "a full {theme:?} theme must not cover the desktop outside the panel"
        );
    }
}
