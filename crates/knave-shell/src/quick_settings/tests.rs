use super::*;
use knave_ui::{TextSpan, TextStyle};
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

fn app() -> QuickSettings {
    let mut app = QuickSettings::new();
    app.apply(QuickSettingsSnapshot {
        open: true,
        network: Some(Network {
            ethernet: Some("enp1s0".into()),
            ..Default::default()
        }),
        output: Some(Volume {
            percent: 65,
            muted: false,
        }),
        input: Some(Volume {
            percent: 40,
            muted: false,
        }),
        brightness: Some(80),
        power: Some(Power {
            active: "balanced".into(),
            available: vec![
                "power-saver".into(),
                "balanced".into(),
                "performance".into(),
            ],
        }),
        battery: Some(Battery {
            name: "BAT1".into(),
            percent: 80,
            status: "Not charging".into(),
            driver_limit: Some(80),
            configured_limit: Some(80),
            charge_limit: Some(ChargeLimit {
                supported: true,
                enabled: true,
                start_threshold: Some(75),
                end_threshold: Some(80),
            }),
        }),
        ..Default::default()
    });
    app
}

#[test]
fn selected_theme_survives_quick_settings_scene_recomposition() {
    let mut light = QuickSettings::with_theme(Theme::Light);
    assert_eq!(light.scene.theme(), Theme::Light);
    light.apply(QuickSettingsSnapshot {
        open: true,
        light: true,
        ..Default::default()
    });
    light.frame([1280.0, 800.0], &mut Metrics);
    assert_eq!(light.scene.theme(), Theme::Light);

    let mut dark = QuickSettings::new();
    assert_eq!(dark.scene.theme(), Theme::Dark);
    dark.apply(QuickSettingsSnapshot {
        open: true,
        ..Default::default()
    });
    dark.frame([1280.0, 800.0], &mut Metrics);
    assert_eq!(dark.scene.theme(), Theme::Dark);
}
#[test]
fn all_pages_fit_narrow_and_short_outputs() {
    for size in [[320.0, 240.0], [320.0, 568.0], [1280.0, 800.0]] {
        for page in [Page::Main, Page::Network, Page::Night, Page::Battery] {
            let mut app = app();
            app.page = page;
            let list = app.frame(size, &mut Metrics).clone();
            assert_eq!(list.clear_color.alpha, 0);
            assert_eq!(app.scene.theme(), app.theme);
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
            {
                let id = CLOSE;
                let r = app.scene.bounds(ElementId(id)).unwrap();
                assert!(r.x >= panel.x && r.x + r.width <= panel.x + panel.width + 0.1);
                assert!(r.y + r.height <= panel.y + panel.height + 0.1);
            }
        }
    }
}
#[test]
fn ethernet_is_preferred_and_volume_values_remain_local_until_commit() {
    let mut app = app();
    let mut n = Network {
        ethernet: Some("enp1s0".into()),
        wireless: Some("Home".into()),
        ..Default::default()
    };
    assert!(n.summary().starts_with("Ethernet"));
    n.ethernet = None;
    assert!(n.summary().starts_with("Home"));
    app.value(ElementId(OUTPUT), 72.0, false);
    assert_eq!(app.state.output.unwrap().percent, 72);
    app.page = Page::Night;
    app.night.enabled = true;
    app.value(ElementId(TEMPERATURE), 3500.0, false);
    assert_eq!(app.night.temperature, 3500);
}

#[test]
fn close_intent_suppresses_late_open_snapshots_until_acknowledged() {
    let mut app = app();
    app.send(QuickSettingsCommand::Close);
    let late = QuickSettingsSnapshot {
        open: true,
        ..Default::default()
    };
    app.apply(late.clone());
    assert!(!app.surface_visible());
    app.apply(QuickSettingsSnapshot::default());
    app.apply(late);
    assert!(app.surface_visible());
}

fn click(app: &mut QuickSettings, id: u64) {
    let r = app.scene.bounds(ElementId(id)).unwrap();
    let p = [r.x + r.width / 2.0, r.y + r.height / 2.0];
    app.input(Input::PointerMove(p));
    app.input(Input::PointerDown(p));
    app.input(Input::PointerUp(p));
    app.frame(app.size, &mut Metrics);
}
#[test]
fn power_is_main_header_only_and_refresh_is_absent() {
    for page in [Page::Main, Page::Network, Page::Night, Page::Battery] {
        let mut app = app();
        app.page = page;
        let list = app.frame([1280.0, 800.0], &mut Metrics).clone();
        assert!(
            !list
                .commands
                .iter()
                .any(|c| matches!(c,knave_ui::DisplayCommand::Text{text,..} if text=="Refresh"))
        );
        assert_eq!(
            app.scene.bounds(ElementId(POWER)).is_some(),
            page == Page::Main
        );
        let close = app.scene.bounds(ElementId(CLOSE)).unwrap();
        if page == Page::Main {
            let power = app.scene.bounds(ElementId(POWER)).unwrap();
            assert_eq!(power.y, close.y);
            assert_eq!(power.height, close.height);
        } else {
            let back = app.scene.bounds(ElementId(BACK)).unwrap();
            assert_eq!(back.y, close.y);
            assert_eq!(back.height, close.height);
        }
    }
}
#[test]
fn power_confirmation_is_modal_survives_updates_and_escape_cancels() {
    let mut app = app();
    app.frame([1280.0, 800.0], &mut Metrics);
    app.select_power(2);
    app.frame(app.size, &mut Metrics);
    assert!(app.confirmation.is_some());
    let dialog = app.scene.bounds(ElementId(CONFIRMATION)).unwrap();
    assert!(dialog.width > 0.0 && dialog.height > 0.0);
    let latest = app.state.clone();
    app.apply(latest);
    app.frame(app.size, &mut Metrics);
    assert!(app.scene.bounds(ElementId(CONFIRMATION)).unwrap().height > 0.0);
    assert!(app.scene.set_focus(Some(ElementId(CLOSE))).is_err());
    app.input(Input::Key {
        key: Key::Escape,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    });
    app.frame(app.size, &mut Metrics);
    assert!(app.confirmation.is_none());
    assert!(app.surface_visible());
    app.select_power(3);
    app.frame(app.size, &mut Metrics);
    click(&mut app, CANCEL_CONFIRMATION);
    assert!(app.confirmation.is_none());
    assert!(app.surface_visible());
}
#[test]
fn brightness_home_reaches_one_percent_and_authentication_releases_input() {
    let mut app = app();
    app.frame([1280.0, 800.0], &mut Metrics);
    app.scene.set_focus(Some(ElementId(BRIGHTNESS))).unwrap();
    app.input(Input::Key {
        key: Key::Home,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    });
    assert_eq!(app.state.brightness, Some(1));
    app.authenticating = true;
    assert!(!app.surface_visible());
    assert_eq!(app.keyboard_mode(), Some(KeyboardMode::None));
    assert!(app.input_region().unwrap().is_empty());
    app.input(Input::FocusLost);
    assert!(app.state.open);
}

#[test]
fn authentication_unmap_suppresses_late_focus_loss_then_restores_normal_close() {
    let mut app = app();
    app.page = Page::Battery;
    app.frame([1280.0, 800.0], &mut Metrics);
    app.authenticating = true;
    app.charge_feedback = ChargeFeedback::Applying(95);
    app.surface_unmapped();

    app.finish_charge_limit(Ok(95));
    assert!(!app.authenticating);
    app.input(Input::FocusLost);
    assert!(app.state.open);
    assert!(!app.auth_unmap_focus_loss);

    app.input(Input::FocusGained);
    app.input(Input::FocusLost);
    assert!(!app.state.open);
}

#[test]
fn fast_auth_result_without_unmap_does_not_suppress_real_focus_loss() {
    let mut app = app();
    app.page = Page::Battery;
    app.frame([1280.0, 800.0], &mut Metrics);
    app.authenticating = true;
    app.charge_feedback = ChargeFeedback::Applying(95);

    app.finish_charge_limit(Ok(95));
    assert!(!app.auth_unmap_focus_loss);
    app.input(Input::FocusLost);
    assert!(!app.state.open);
}

#[test]
fn focus_loss_during_auth_consumes_unmap_guard_without_closing() {
    let mut app = app();
    app.page = Page::Battery;
    app.frame([1280.0, 800.0], &mut Metrics);
    app.authenticating = true;
    app.surface_unmapped();

    app.input(Input::FocusLost);
    assert!(app.state.open);
    assert!(!app.auth_unmap_focus_loss);

    app.authenticating = false;
    app.input(Input::FocusGained);
    app.input(Input::FocusLost);
    assert!(!app.state.open);
}

#[test]
fn focus_gained_clears_unconsumed_auth_unmap_guard() {
    let mut app = app();
    app.frame([1280.0, 800.0], &mut Metrics);
    app.authenticating = true;
    app.surface_unmapped();
    app.authenticating = false;

    app.input(Input::FocusGained);
    assert!(!app.auth_unmap_focus_loss);
    app.input(Input::FocusLost);
    assert!(!app.state.open);
}

#[test]
fn trimmed_subtitles_keep_status_and_charge_policy_text() {
    let mut app = app();
    app.page = Page::Network;
    let network = app.frame([1280.0, 800.0], &mut Metrics).clone();
    assert!(!network.commands.iter().any(|command| matches!(command,
        knave_ui::DisplayCommand::Text { text, .. }
            if text.contains("Wi-Fi remains available"))));
    assert!(network.commands.iter().any(|command| matches!(command,
        knave_ui::DisplayCommand::Text { text, .. }
            if text.contains("Ethernet"))));
    app.page = Page::Battery;
    app.dirty = true;
    let battery = app.frame(app.size, &mut Metrics).clone();
    assert!(!battery.commands.iter().any(|command| matches!(command,
        knave_ui::DisplayCommand::Text { text, .. }
            if text.contains("Stop charging at this level"))));
    assert!(battery.commands.iter().any(|command| matches!(command,
        knave_ui::DisplayCommand::Text { text, .. }
            if text.contains("administrator authentication"))));
}

#[test]
fn compact_root_keeps_device_controls_visible_and_tiles_side_by_side() {
    let mut app = app();
    app.frame([1280.0, 800.0], &mut Metrics);
    let panel = app.scene.bounds(PANEL).unwrap();
    assert_eq!(panel.width, 340.0);
    let target = |predicate: fn(&Target) -> bool| {
        app.targets
            .iter()
            .find(|(_, t)| predicate(t))
            .map(|(id, _)| app.scene.bounds(*id).unwrap())
            .unwrap()
    };
    let network = target(|t| matches!(t, Target::Page(Page::Network)));
    let bluetooth = target(|t| {
        matches!(
            t,
            Target::Command(QuickSettingsCommand::SetBluetooth { .. })
        )
    });
    assert_eq!(network.y, bluetooth.y);
    assert!(network.x + network.width <= bluetooth.x);
    assert!(app.scene.bounds(ElementId(PROFILE)).unwrap().y < network.y);
    for rect in [
        app.scene.bounds(ElementId(OUTPUT)).unwrap(),
        app.scene.bounds(ElementId(INPUT)).unwrap(),
        app.scene.bounds(ElementId(BRIGHTNESS)).unwrap(),
        target(|t| matches!(t, Target::Page(Page::Battery))),
    ] {
        assert!(
            rect.y >= panel.y && rect.y + rect.height <= panel.y + panel.height,
            "{rect:?} outside {panel:?}"
        );
    }
    let panel_surface = app
        .scene
        .computed_style(PANEL)
        .unwrap()
        .style
        .background
        .expect("quick-settings card should receive its theme surface");
    assert!(panel_surface.fill.is_some_and(|fill| fill.alpha > 0));
    click(&mut app, POWER);
    assert!(app.scene.interaction(ElementId(POWER)).unwrap().expanded);
}
#[test]
fn profile_menu_and_bundled_icons_have_bounded_reusable_state() {
    let mut app = app();
    app.frame([1280.0, 800.0], &mut Metrics);
    assert!(
        matches!(app.scene.widget(ElementId(PROFILE)),Some(Widget::Menu{items,..}) if items.len()==3)
    );
    for light in [false, true] {
        app.theme_policy_changed((if light { Theme::Light } else { Theme::Dark }).into(), "")
            .expect("quick-settings theme should apply");
        app.state.light = light;
        app.apply(app.state.clone());
        let keys = |list: &DisplayList| {
            list.commands
                .iter()
                .filter_map(|c| match c {
                    knave_ui::DisplayCommand::Image { image, .. } => {
                        assert_eq!(image.pixels().len(), 24 * 24 * 4);
                        Some(image.cache_key())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let first = keys(app.frame([1280.0, 800.0], &mut Metrics));
        app.dirty = true;
        let next = keys(app.frame([1280.0, 800.0], &mut Metrics));
        assert!(!first.is_empty());
        assert_eq!(first, next);
    }
}

#[test]
fn row_icons_are_vertically_centered() {
    let mut app = app();
    app.frame([1280.0, 800.0], &mut Metrics);
    let tree = app.compose();
    fn find(e: &Element, id: ElementId) -> Option<&Element> {
        if e.id == id {
            return Some(e);
        }
        e.children.iter().find_map(|c| find(c, id))
    }
    let network = *app
        .targets
        .iter()
        .find(|(_, t)| matches!(t, Target::Page(Page::Network)))
        .unwrap()
        .0;
    let bluetooth = *app
        .targets
        .iter()
        .find(|(_, t)| {
            matches!(
                t,
                Target::Command(QuickSettingsCommand::SetBluetooth { .. })
            )
        })
        .unwrap()
        .0;
    for owner in [ElementId(POWER), network, bluetooth] {
        let icon = find(&tree, owner)
            .unwrap()
            .children
            .iter()
            .find(|e| matches!(e.widget, Widget::Image(..)))
            .unwrap();
        let child = app.scene.bounds(icon.id).unwrap();
        let parent = app.scene.bounds(owner).unwrap();
        assert!(
            (child.y + child.height / 2.0 - parent.y - parent.height / 2.0).abs() < 0.1,
            "{owner:?}: {child:?} in {parent:?}"
        );
    }
}

#[test]
fn charge_limit_completion_reports_driver_acceptance_and_preserves_page() {
    let mut app = app();
    app.page = Page::Battery;
    app.charge_feedback = ChargeFeedback::Applying(95);
    app.authenticating = true;
    app.finish_charge_limit(Ok(95));
    assert!(matches!(app.charge_feedback, ChargeFeedback::Applied(95)));
    assert_eq!(
        app.state.battery.as_ref().unwrap().configured_limit,
        Some(95)
    );
    assert_eq!(app.state.battery.as_ref().unwrap().driver_limit, Some(80));
    assert_eq!(app.limit, 95);
    assert!(app.page == Page::Battery);
    assert!(!app.authenticating);
    let list = app.frame([1280.0, 800.0], &mut Metrics).clone();
    assert!(list.commands.iter().any(|c|matches!(c,knave_ui::DisplayCommand::Text{text,..} if text.contains("Driver accepted the requested 95% limit"))));
    assert!(list.commands.iter().any(|c|matches!(c,knave_ui::DisplayCommand::Text{text,..} if text.contains("Driver-reported threshold · 80%"))));
    app.charge_feedback = ChargeFeedback::Applying(85);
    app.authenticating = true;
    app.finish_charge_limit(Err("Authentication cancelled".into()));
    assert!(matches!(app.charge_feedback, ChargeFeedback::Failed(_)));
    assert_eq!(
        app.state.battery.as_ref().unwrap().configured_limit,
        Some(95)
    );
    assert!(!app.authenticating);
    assert!(app.surface_visible());

    app.charge_feedback = ChargeFeedback::Applying(95);
    app.finish_charge_limit(Ok(90));
    assert!(
        matches!(app.charge_feedback, ChargeFeedback::Failed(ref error) if error.contains("Requested 95%, but driver reports 90%"))
    );
    assert_eq!(
        app.state.battery.as_ref().unwrap().configured_limit,
        Some(95)
    );
}
#[test]
fn charge_limit_without_service_never_claims_applied() {
    let mut app = app();
    app.page = Page::Battery;
    app.apply_charge_limit();
    assert!(matches!(app.charge_feedback, ChargeFeedback::Failed(_)));
    assert!(!app.authenticating);
    assert!(app.surface_visible());
}

#[test]
fn charge_limit_initial_value_edit_and_pending_state_are_explicit() {
    for (configured, driver, expected) in [
        (Some(85), Some(80), 85),
        (Some(95), Some(80), 95),
        (Some(100), Some(80), 100),
        (None, Some(85), 85),
        (None, None, 100),
    ] {
        let mut sample = app();
        let battery = sample.state.battery.as_mut().unwrap();
        battery.configured_limit = configured;
        battery.driver_limit = driver;
        let snapshot = sample.state.clone();
        sample.apply(snapshot);
        assert_eq!(sample.limit, expected);
    }

    let mut app = app();
    app.page = Page::Battery;
    app.value(ElementId(LIMIT), 95.0, true);
    assert_eq!(app.limit, 95);
    assert!(app.limit_edited);
    assert!(matches!(app.charge_feedback, ChargeFeedback::Idle));
    let mut fresh = app.state.clone();
    fresh.battery.as_mut().unwrap().configured_limit = Some(85);
    app.apply(fresh);
    assert_eq!(app.limit, 95);

    app.charge_feedback = ChargeFeedback::Applying(95);
    app.dirty = true;
    let pending = app.frame([1280.0, 800.0], &mut Metrics).clone();
    assert!(pending.commands.iter().any(|c|matches!(c,knave_ui::DisplayCommand::Text{text,..} if text.contains("Applying 95% limit"))));
    fn find(e: &Element, id: ElementId) -> Option<&Element> {
        if e.id == id {
            return Some(e);
        }
        e.children.iter().find_map(|child| find(child, id))
    }
    let tree = app.compose();
    assert!(!find(&tree, ElementId(LIMIT)).unwrap().enabled);
    assert!(app.scene.bounds(ElementId(LIMIT)).is_some());
}

#[test]
fn custom_limit_summary_and_support_do_not_follow_upower_preset_toggle() {
    let mut app = app();
    app.state.battery.as_mut().unwrap().configured_limit = Some(95);
    app.state
        .battery
        .as_mut()
        .unwrap()
        .charge_limit
        .as_mut()
        .unwrap()
        .enabled = false;
    let main = app.frame([1280.0, 800.0], &mut Metrics).clone();
    assert!(main.commands.iter().any(|c|matches!(c,knave_ui::DisplayCommand::Text{text,..} if text.contains("Charge limit · 95%"))));

    app.page = Page::Battery;
    app.state.battery.as_mut().unwrap().driver_limit = None;
    app.dirty = true;
    let battery_page = app.frame([1280.0, 800.0], &mut Metrics).clone();
    let text = battery_page
        .commands
        .iter()
        .filter_map(|command| match command {
            knave_ui::DisplayCommand::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let selected = text
        .iter()
        .position(|value| value.contains("Selected limit"));
    let upower = text
        .iter()
        .position(|value| value.contains("UPower preset disabled"));
    assert!(selected.is_some_and(|selected| upower.is_some_and(|upower| selected < upower)));
    assert!(!text.iter().any(|value| value.contains("Start at 75%")));
    assert!(!text.iter().any(|value| value.contains("stop at 80%")));
    fn find(e: &Element, id: ElementId) -> Option<&Element> {
        if e.id == id {
            return Some(e);
        }
        e.children.iter().find_map(|child| find(child, id))
    }
    let tree = app.compose();
    assert!(!find(&tree, ElementId(LIMIT)).unwrap().enabled);

    app.state.battery.as_mut().unwrap().driver_limit = Some(95);
    app.state
        .battery
        .as_mut()
        .unwrap()
        .charge_limit
        .as_mut()
        .unwrap()
        .supported = false;
    app.dirty = true;
    app.frame([1280.0, 800.0], &mut Metrics);
    let tree = app.compose();
    assert!(find(&tree, ElementId(LIMIT)).unwrap().enabled);
}

#[test]
fn shell_panels_do_not_add_decorative_outlines() {
    let mut app = app();
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
        let mut app = app();
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
