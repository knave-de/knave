use super::view::DOCK_ELEMENT_ID_BASE;
use super::*;
use knave_desktop_api::{WindowSummary, WorkspaceSummary};
use knave_shell_ui::UiNode;
use knave_ui::{TextSpan, TextStyle};
struct Metrics;
impl TextMeasurer for Metrics {
    fn measure(&mut self, text: &str, style: &TextStyle, width: f32) -> TextLayout {
        TextLayout {
            size: [(text.len() as f32 * 7.0).min(width), style.line_height],
            carets: vec![],
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
fn snapshot() -> DesktopSnapshot {
    DesktopSnapshot {
        generation: 7,
        overview_visible: true,
        workspaces: (1..=3)
            .map(|i| WorkspaceSummary {
                workspace: WorkspaceId(i),
                active: i == 2,
                window_count: 4,
                visible_window_count: 1,
            })
            .collect(),
        windows: (1..=14)
            .map(|id| WindowSummary {
                id: WindowId(id),
                title: format!("Window {id}"),
                app_id: "Editor".into(),
                workspace: WorkspaceId(if id == 14 { 3 } else { 2 }),
                focused: id == 1,
                minimized: id != 1,
                floating: false,
                fullscreen: false,
                maximized: false,
            })
            .collect(),
    }
}
/// Removes a fixture directory when the owning test thread ends.
struct TempDir(std::path::PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
thread_local! {
    static FIXTURES: std::cell::RefCell<Vec<TempDir>> = const { std::cell::RefCell::new(Vec::new()) };
}
fn fixture_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let root = std::env::temp_dir().join(format!(
        "knave-shell-{label}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    FIXTURES.with(|f| f.borrow_mut().push(TempDir(root.clone())));
    root
}
/// Twelve "Tool NN" entries exercise paging; "Editor" checks Exec expansion.
/// Tool 12 names an icon that does not exist, to exercise the fallback.
fn apps_root() -> std::path::PathBuf {
    let root = fixture_dir("apps");
    let dir = root.join("applications");
    std::fs::create_dir_all(&dir).unwrap();
    let entry = |name: &str, exec: &str, icon: &str| {
        format!(
            "[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\nIcon={icon}\nComment=About {name}\n"
        )
    };
    std::fs::write(
        dir.join("editor.desktop"),
        entry("Editor", "editor --new %U", "editor-icon"),
    )
    .unwrap();
    for n in 1..=12 {
        let icon = if n == 12 {
            "ghost".into()
        } else {
            format!("tool-icon-{n:02}")
        };
        std::fs::write(
            dir.join(format!("tool{n:02}.desktop")),
            entry(&format!("Tool {n:02}"), &format!("tool{n:02}"), &icon),
        )
        .unwrap();
    }
    root
}
fn apps_environment() -> knave_apps::Environment {
    knave_apps::Environment {
        data_dirs: vec![apps_root()],
        ..Default::default()
    }
}
fn catalog() -> knave_apps::Catalog {
    knave_apps::Catalog::load_with(&apps_environment()).unwrap()
}
fn png_bytes(rgba: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, 48, 48);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer
        .write_image_data(&(0..48 * 48).flat_map(|_| rgba).collect::<Vec<u8>>())
        .unwrap();
    writer.finish().unwrap();
    out
}
/// A one-directory theme holding an icon for each fixture app plus the fallback.
fn icon_lookup(with_fallback: bool) -> knave_icons::IconLookup {
    let root = fixture_dir("icons");
    let theme = root.join("hicolor");
    std::fs::create_dir_all(theme.join("48x48/apps")).unwrap();
    std::fs::write(
        theme.join("index.theme"),
        "[Icon Theme]\nDirectories=48x48/apps\n[48x48/apps]\nSize=48\nType=Fixed\n",
    )
    .unwrap();
    let mut names: Vec<String> = (1..=11).map(|n| format!("tool-icon-{n:02}")).collect();
    names.push("editor-icon".into());
    if with_fallback {
        names.push("application-x-executable".into());
    }
    for name in names {
        std::fs::write(
            theme.join(format!("48x48/apps/{name}.png")),
            png_bytes([0, 200, 0, 255]),
        )
        .unwrap();
    }
    knave_icons::IconLookup::new(vec![root], vec![])
}
fn app() -> Overview {
    let mut app = Overview::new()
        .with_catalog(catalog())
        .with_icon_lookup(icon_lookup(true));
    app.desktop_snapshot(&snapshot());
    app.frame([1200.0, 800.0], &mut Metrics);
    app
}
fn dock_app() -> Overview {
    let mut app = Overview::new()
        .with_catalog(catalog())
        .with_icon_lookup(icon_lookup(true))
        .with_pinned_apps(vec!["tool01.desktop".into(), "editor.desktop".into()]);
    let mut desktop = snapshot();
    desktop.windows.retain(|window| window.id.0 != 14);
    desktop.windows.push(WindowSummary {
        id: WindowId(14),
        title: "Other workspace".into(),
        app_id: "Other Workspace App".into(),
        workspace: WorkspaceId(3),
        focused: false,
        minimized: false,
        floating: false,
        fullscreen: false,
        maximized: false,
    });
    desktop.windows.push(WindowSummary {
        id: WindowId(15),
        title: "Minimized tool".into(),
        app_id: "Tool 03".into(),
        workspace: WorkspaceId(2),
        focused: false,
        minimized: true,
        floating: false,
        fullscreen: false,
        maximized: false,
    });
    app.desktop_snapshot(&desktop);
    app.frame([1200.0, 800.0], &mut Metrics);
    app
}
fn key(app: &mut Overview, key: Key, pressed: bool) {
    app.input(Input::Key {
        key,
        pressed,
        repeat: false,
        modifiers: KeyModifiers::default(),
    });
}
fn click(app: &mut Overview, id: ElementId) {
    let r = app.scene.bounds(id).unwrap();
    let p = [r.x + r.width / 2.0, r.y + r.height / 2.0];
    app.input(Input::PointerDown(p));
    app.input(Input::PointerUp(p));
}
fn publish_workspace(app: &mut Overview, workspace: WorkspaceId) {
    let mut next = app.snapshot().unwrap().clone();
    for w in &mut next.workspaces {
        w.active = w.workspace == workspace;
    }
    app.desktop_snapshot(&next);
    app.frame([1200.0, 800.0], &mut Metrics);
}
#[test]
fn minimized_shelf_is_workspace_scoped_and_paginated() {
    let mut app = app();
    assert!(app.scene.bounds(app.window_ids[&WindowId(1)]).is_none());
    assert!(app.scene.bounds(app.window_ids[&WindowId(14)]).is_none());
    assert!(app.scene.bounds(app.window_ids[&WindowId(2)]).is_some());
    let previous = app.scene.bounds(ElementId(30)).unwrap();
    let next = app.scene.bounds(ElementId(31)).unwrap();
    assert!(
        previous.x < next.x,
        "previous: {previous:?}, next: {next:?}"
    );
    click(&mut app, ElementId(31));
    app.frame([1200.0, 800.0], &mut Metrics);
    assert!(app.scene.bounds(app.window_ids[&WindowId(2)]).is_none());
    assert!(app.scene.bounds(app.window_ids[&WindowId(7)]).is_some());
}

#[test]
fn single_icon_tray_uses_only_the_visible_icon_width() {
    let mut app = app();
    let mut next = app.snapshot().unwrap().clone();
    for window in &mut next.windows {
        window.minimized = window.id == WindowId(2);
    }
    app.desktop_snapshot(&next);
    app.frame([1200.0, 800.0], &mut Metrics);

    let tray = app.scene.bounds(tray_id(2)).unwrap();
    let icon = app.scene.bounds(app.window_ids[&WindowId(2)]).unwrap();
    assert!(
        tray.width <= icon.width + 13.0,
        "tray: {tray:?}, icon: {icon:?}"
    );
}

/// Element id of the minimized icon tray docked in one workspace card.
fn tray_id(ws: u64) -> ElementId {
    ElementId(0x100_0000_0000 + ws * 32 + 3)
}
fn tooltip_visible(app: &Overview, label: &str) -> bool {
    app.display.commands.iter().any(|command| match command {
        knave_ui::DisplayCommand::Text { text, .. } => text == label,
        _ => false,
    })
}
#[test]
fn the_minimized_tray_is_icon_only_in_the_lower_left_of_the_selected_card() {
    let mut app = app();
    settle(&mut app, [1200.0, 800.0]);
    let card = app
        .scene
        .bounds(ElementId(0x100_0000_0000 + 2 * 32))
        .unwrap();
    let tray = app
        .scene
        .bounds(tray_id(2))
        .expect("tray on the selected card");
    assert!(tray.height <= 50.0 && tray.height >= 30.0);
    assert!(tray.width < card.width);
    assert!((tray.x - (card.x + 12.0)).abs() < 1.0);
    assert!(((card.y + card.height) - (tray.y + tray.height) - 12.0).abs() < 1.0);
    // Entries are small icon buttons inside the tray, not heading/chip cards.
    let icon = app.scene.bounds(app.window_ids[&WindowId(2)]).unwrap();
    assert!(icon.width <= 40.0 && icon.height <= 40.0);
    assert!(icon.x >= tray.x && icon.y >= tray.y);
    assert!(icon.x + icon.width <= tray.x + tray.width + 0.5);
    assert!(icon.y + icon.height <= tray.y + tray.height + 0.5);
    for other in [tray_id(1), tray_id(3)] {
        assert!(app.scene.bounds(other).is_none(), "tray off selected card");
    }
    // Only the current page is composed in the tray.
    assert!(app.scene.bounds(app.window_ids[&WindowId(12)]).is_none());
    let list = app.frame([1200.0, 800.0], &mut Metrics).clone();
    assert!(list.commands.iter().any(|command| matches!(
        command,
        knave_ui::DisplayCommand::Image { bounds, clip, .. }
            if bounds.x >= tray.x
                && bounds.y >= tray.y
                && bounds.x + bounds.width <= tray.x + tray.width
                && bounds.y + bounds.height <= tray.y + tray.height
                && clip.is_some()
    )));
    // No heading, no app id or window title subtitles, no empty-shelf note.
    for command in &list.commands {
        if let knave_ui::DisplayCommand::Text { text, .. } = command {
            assert!(!text.contains("Minimized"), "{text:?}");
            assert!(!text.contains("Editor"), "{text:?}");
        }
    }
}
#[test]
fn tray_icons_name_the_exact_window_on_hover() {
    let mut app = app();
    let point = |app: &Overview, id: ElementId| {
        let r = app.scene.bounds(id).unwrap();
        [r.x + r.width / 2.0, r.y + r.height / 2.0]
    };
    for id in 2..=6 {
        app.input(Input::PointerMove(point(
            &app,
            app.window_ids[&WindowId(id)],
        )));
        app.frame([1200.0, 800.0], &mut Metrics);
        // Same-application windows are still told apart.
        assert!(
            tooltip_visible(&app, &format!("Restore Window {id}")),
            "window {id}"
        );
    }
    assert!(!tooltip_visible(&app, "Restore Window 7"));
    click(&mut app, ElementId(31));
    app.frame([1200.0, 800.0], &mut Metrics);
    assert!(app.scene.bounds(app.window_ids[&WindowId(8)]).is_some());
    app.input(Input::PointerMove(point(
        &app,
        app.window_ids[&WindowId(8)],
    )));
    app.frame([1200.0, 800.0], &mut Metrics);
    assert!(tooltip_visible(&app, "Restore Window 8"));
    app.input(Input::PointerLeave);
    app.frame([1200.0, 800.0], &mut Metrics);
    assert!(!tooltip_visible(&app, "Restore Window 8"));
}
#[test]
fn keyboard_navigation_reaches_tray_icons_and_restores() {
    let mut app = app();
    let mut focused = None;
    for _ in 0..64 {
        key(&mut app, Key::Tab, true);
        key(&mut app, Key::Tab, false);
        let id = match app.scene.focus() {
            Some(id) => id,
            None => continue,
        };
        if matches!(app.targets.get(&id), Some(Target::Window(_))) {
            focused = Some(id);
            break;
        }
    }
    let id = focused.expect("Tab reaches a tray icon");
    key(&mut app, Key::Enter, true);
    key(&mut app, Key::Enter, false);
    let expected = match app.targets.get(&id) {
        Some(Target::Window(window)) => *window,
        _ => unreachable!("checked above"),
    };
    assert!(matches!(
        app.take_request(),
        Some(HostRequest::Desktop(DesktopCommand::RestoreWindow { window }))
            if window == expected
    ));
}
#[test]
fn minimized_icons_load_without_typing_a_query() {
    let mut app = Overview::new()
        .with_catalog_environment(apps_environment())
        .with_icon_lookup(icon_lookup(true));
    app.desktop_snapshot(&snapshot());
    app.frame([1200.0, 800.0], &mut Metrics);
    settle(&mut app, [1200.0, 800.0]);
    assert!(app.query.text().is_empty());
    assert!(matches!(app.apps, Apps::Ready(_)));
    assert!(app.icons.get("editor-icon").is_some());
    let wid = app.window_ids[&WindowId(2)].0;
    assert!(app.scene.bounds(ElementId(0x500_0000_0000 + wid)).is_some());
}

#[test]
fn missing_theme_icons_show_a_builtin_tray_glyph() {
    let mut app = Overview::new()
        .with_catalog(catalog())
        .with_icon_lookup(icon_lookup(false));
    let mut next = snapshot();
    for window in &mut next.windows {
        window.app_id = "Mystery".into();
    }
    app.desktop_snapshot(&next);
    app.frame([1200.0, 800.0], &mut Metrics);
    settle(&mut app, [1200.0, 800.0]);

    assert!(app.display.commands.iter().any(|command| matches!(
        command,
        knave_ui::DisplayCommand::Text { text, .. } if text == "▣"
    )));
}
#[test]
fn unknown_minimized_apps_fall_back_to_the_generic_icon() {
    let mut app = app();
    let mut next = app.snapshot().unwrap().clone();
    next.windows[1].app_id = "Mystery".into();
    app.desktop_snapshot(&next);
    app.frame([1200.0, 800.0], &mut Metrics);
    settle(&mut app, [1200.0, 800.0]);
    // The worker answers an unknown application with the theme fallback.
    assert!(app.icons.get("application-x-executable").is_some());
    assert!(app.error.is_none());
    assert!(app.scene.bounds(app.window_ids[&WindowId(2)]).is_some());
}
#[test]
fn no_icon_tray_when_the_selected_workspace_has_no_minimized_windows() {
    let mut app = Overview::new()
        .with_catalog(catalog())
        .with_icon_lookup(icon_lookup(true));
    let mut next = snapshot();
    for window in &mut next.windows {
        window.minimized = false;
    }
    app.desktop_snapshot(&next);
    app.frame([1200.0, 800.0], &mut Metrics);
    assert!(app.scene.bounds(tray_id(2)).is_none());
    assert!(!app.display.commands.iter().any(|command| matches!(
        command,
        knave_ui::DisplayCommand::Text { text, .. }
            if text == "No minimized windows"
    )));
    assert!(app.error.is_none());
    assert!(
        app.panes
            .iter()
            .any(|pane| pane.workspace == WorkspaceId(2))
    );
}
#[test]
fn navigation_switches_workspace_without_hiding_and_enter_returns_to_it() {
    let mut app = app();
    key(&mut app, Key::Right, true);
    assert_eq!(app.workspace, Some(WorkspaceId(2)));
    assert!(matches!(
        app.take_request(),
        Some(HostRequest::Desktop(DesktopCommand::FocusWorkspace {
            workspace: WorkspaceId(3)
        }))
    ));
    // A snapshot can precede the command acknowledgement.
    publish_workspace(&mut app, WorkspaceId(3));
    app.desktop_action_finished(Ok(()));
    assert!(app.take_request().is_none());
    assert!(app.surface_visible());
    assert_eq!(app.workspace, Some(WorkspaceId(3)));
    assert_eq!(status_title(&app), None);
    assert!(app.scene.bounds(app.window_ids[&WindowId(14)]).is_some());
    key(&mut app, Key::Enter, true);
    key(&mut app, Key::Enter, false);
    assert!(matches!(
        app.take_request(),
        Some(HostRequest::Desktop(DesktopCommand::FocusWorkspace {
            workspace: WorkspaceId(3)
        }))
    ));
    app.desktop_action_finished(Ok(()));
    assert!(matches!(
        app.take_request(),
        Some(HostRequest::Desktop(DesktopCommand::SetOverviewVisible {
            visible: false
        }))
    ));
}
#[test]
fn failed_navigation_and_repeated_input_preserve_confirmed_workspace() {
    let mut app = app();
    key(&mut app, Key::Right, true);
    let request = app.take_request();
    assert!(matches!(
        request,
        Some(HostRequest::Desktop(DesktopCommand::FocusWorkspace { .. }))
    ));
    key(&mut app, Key::Left, true);
    assert!(app.take_request().is_none());
    assert!(!app.hide_after_action);
    app.desktop_action_finished(Err("Workspace unavailable".into()));
    assert_eq!(app.workspace, Some(WorkspaceId(2)));
    assert_eq!(app.error.as_deref(), Some("Workspace unavailable"));
    assert!(app.take_request().is_none());
    assert!(app.surface_visible());
    key(&mut app, Key::Left, true);
    assert!(matches!(
        app.take_request(),
        Some(HostRequest::Desktop(DesktopCommand::FocusWorkspace {
            workspace: WorkspaceId(1)
        }))
    ));
    // The acknowledgement can also precede the pushed snapshot.
    app.desktop_action_finished(Ok(()));
    assert!(app.take_request().is_none());
    publish_workspace(&mut app, WorkspaceId(1));
    assert_eq!(app.workspace, Some(WorkspaceId(1)));
    assert!(app.surface_visible());
}
#[test]
fn workspace_controls_switch_without_hiding() {
    for id in [
        ElementId(21),
        ElementId(0x100_0000_0000 + 3 * 32),
        bar_target(
            &app(),
            knave_shell_ui::UiAction::FocusWorkspace(WorkspaceId(3)),
        ),
    ] {
        let mut app = app();
        // A side card may be clipped; activating its target exercises the same action.
        app.activate(id);
        assert!(matches!(
            app.take_request(),
            Some(HostRequest::Desktop(DesktopCommand::FocusWorkspace {
                workspace: WorkspaceId(3)
            }))
        ));
        app.desktop_action_finished(Ok(()));
        assert!(app.take_request().is_none());
        publish_workspace(&mut app, WorkspaceId(3));
        assert_eq!(app.workspace, Some(WorkspaceId(3)));
        assert_eq!(status_title(&app), None);
        assert!(app.surface_visible());
    }
}
#[test]
fn restore_waits_for_ack_and_failure_keeps_overview_open() {
    let mut app = app();
    let id = app.window_ids[&WindowId(2)];
    click(&mut app, id);
    assert!(matches!(
        app.take_request(),
        Some(HostRequest::Desktop(DesktopCommand::RestoreWindow {
            window: WindowId(2)
        }))
    ));
    assert!(!app.should_close());
    click(&mut app, id);
    assert!(app.take_request().is_none());
    app.desktop_action_finished(Err("Window no longer exists".into()));
    assert!(!app.should_close());
    assert!(!app.pending);
    assert!(app.error.is_some());
}
#[test]
fn snapshots_with_same_generation_sync_active_workspace_and_membership() {
    let mut app = app();
    publish_workspace(&mut app, WorkspaceId(3));
    let mut next = snapshot();
    next.windows.retain(|w| w.id != WindowId(2));
    next.workspaces[0].active = true;
    next.workspaces[1].active = false;
    app.desktop_snapshot(&next);
    app.frame([1200.0, 800.0], &mut Metrics);
    assert_eq!(app.workspace, Some(WorkspaceId(1)));
    assert_eq!(status_title(&app), None);
    assert!(!app.window_ids.contains_key(&WindowId(2)));
    next.workspaces.pop();
    app.desktop_snapshot(&next);
    assert_eq!(app.workspace, Some(WorkspaceId(1)));
}
#[test]
fn search_is_unicode_editable_and_escape_hides_overview() {
    let mut app = app();
    app.input(Input::Text("Editor".into()));
    settle(&mut app, [1200.0, 800.0]);
    assert_eq!(app.query.text(), "Editor");
    assert_eq!(app.scene.focus(), Some(SEARCH));
    key(&mut app, Key::Escape, true);
    app.frame([1200.0, 800.0], &mut Metrics);
    assert!(matches!(
        app.take_request(),
        Some(HostRequest::Desktop(DesktopCommand::SetOverviewVisible {
            visible: false
        }))
    ));
    assert!(!app.should_close());
}
#[test]
fn idle_frames_reuse_layout_and_panes_are_bounded_to_neighbors() {
    let mut app = app();
    // The first frame queues icon loads; idleness is only defined once they land.
    settle(&mut app, [1200.0, 800.0]);
    let stats = app.scene.stats();
    assert!(!app.needs_frame());
    app.frame([1200.0, 800.0], &mut Metrics);
    assert_eq!(app.scene.stats(), stats);
    let panes = app.overview_panes().unwrap();
    assert_eq!(panes.len(), 3);
    assert_eq!(
        panes.iter().map(|pane| pane.workspace).collect::<Vec<_>>(),
        vec![WorkspaceId(1), WorkspaceId(3), WorkspaceId(2)]
    );
    assert!(panes.iter().all(|pane| pane.width > 0 && pane.height > 0));
    publish_workspace(&mut app, WorkspaceId(3));
    app.frame([1200.0, 800.0], &mut Metrics);
    assert_eq!(
        app.overview_panes()
            .unwrap()
            .iter()
            .map(|pane| pane.workspace)
            .collect::<Vec<_>>(),
        vec![WorkspaceId(2), WorkspaceId(3)]
    );
}

#[test]
fn layouts_fit_compact_and_portrait_outputs_with_unique_control_ids() {
    for size in [
        [800.0, 600.0],
        [600.0, 900.0],
        [3840.0, 2160.0],
        [320.0, 240.0],
    ] {
        let mut app = app();
        app.frame(size, &mut Metrics);
        assert!(app.error.is_none(), "{size:?}: {:?}", app.error);
        app.input(Input::Text("Tool".into()));
        settle(&mut app, size);
        assert!(app.error.is_none());
        click(&mut app, ElementId(31));
        app.frame(size, &mut Metrics);
        assert!(app.error.is_none());
    }
}
#[test]
fn disconnected_overview_cannot_dispatch_stale_window_actions() {
    let mut app = app();
    app.desktop_unavailable();
    let id = app.window_ids[&WindowId(2)];
    app.activate(id);
    assert!(app.take_request().is_none());
    app.desktop_snapshot(&snapshot());
    assert!(app.connected);
    assert!(app.error.is_none());
}

#[test]
fn disconnection_unmaps_and_cancels_unsent_actions_then_reconnects() {
    let mut app = app();
    assert!(app.surface_visible());
    app.dispatch(DesktopCommand::FocusWorkspace {
        workspace: WorkspaceId(3),
    });
    app.desktop_unavailable();
    assert!(!app.surface_visible());
    assert!(!app.should_close());
    assert!(app.take_request().is_none());
    assert!(!app.pending);
    app.desktop_action_finished(Ok(()));
    assert!(app.take_request().is_none());
    app.desktop_snapshot(&snapshot());
    assert!(app.surface_visible());
}

#[test]
fn completing_activation_while_hidden_does_not_hide_the_next_opening() {
    let mut app = app();
    app.dispatch(DesktopCommand::FocusWorkspace {
        workspace: WorkspaceId(3),
    });
    assert!(app.take_request().is_some());
    let mut hidden = snapshot();
    hidden.overview_visible = false;
    app.desktop_snapshot(&hidden);
    app.desktop_action_finished(Ok(()));
    assert!(!app.pending);
    assert!(app.take_request().is_none());
    app.desktop_snapshot(&snapshot());
    assert!(app.surface_visible());
    assert!(app.take_request().is_none());
}

#[test]
fn disconnected_inflight_action_is_completed_without_a_stale_hide() {
    let mut app = app();
    app.dispatch(DesktopCommand::FocusWorkspace {
        workspace: WorkspaceId(3),
    });
    assert!(app.take_request().is_some());
    app.desktop_unavailable();
    app.desktop_snapshot(&snapshot());
    assert!(app.pending);
    app.desktop_action_finished(Ok(()));
    assert!(!app.pending);
    assert!(app.take_request().is_none());
}

#[test]
fn activation_from_an_earlier_opening_cannot_hide_a_reopened_overview() {
    let mut app = app();
    app.dispatch(DesktopCommand::FocusWorkspace {
        workspace: WorkspaceId(3),
    });
    assert!(app.take_request().is_some());
    let mut hidden = snapshot();
    hidden.overview_visible = false;
    app.desktop_snapshot(&hidden);
    app.desktop_snapshot(&snapshot());
    app.desktop_action_finished(Ok(()));
    assert!(app.surface_visible());
    assert!(app.take_request().is_none());
    assert!(!app.pending);
}

/// Draw frames until the worker has delivered everything it owes; fails instead of hanging.
fn settle(app: &mut Overview, size: [f32; 2]) {
    for _ in 0..5000 {
        app.frame(size, &mut Metrics);
        if !app.is_loading() && !app.needs_frame() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    panic!("loader did not settle");
}
fn type_query(app: &mut Overview, query: &str) {
    app.input(Input::Text(query.into()));
    settle(app, [1200.0, 800.0]);
}
fn spawned(app: &mut Overview) -> Option<Vec<String>> {
    match app.take_request() {
        Some(HostRequest::Desktop(DesktopCommand::Spawn { argv })) => Some(argv),
        _ => None,
    }
}
#[test]
fn enter_launches_the_best_application_match_and_hides_after_ack() {
    let mut app = app();
    type_query(&mut app, "edit");
    key(&mut app, Key::Enter, true);
    key(&mut app, Key::Enter, false);
    assert_eq!(
        spawned(&mut app),
        Some(vec!["editor".into(), "--new".into()])
    );
    assert!(!app.should_close());
    app.desktop_action_finished(Ok(()));
    assert!(matches!(
        app.take_request(),
        Some(HostRequest::Desktop(DesktopCommand::SetOverviewVisible {
            visible: false
        }))
    ));
    app.desktop_action_finished(Ok(()));
    assert!(!app.should_close());
}
#[test]
fn clicking_a_result_launches_it_and_failure_keeps_overview_open() {
    let mut app = app();
    type_query(&mut app, "tool 03");
    let id = app_element(app.search()[0]);
    click(&mut app, id);
    assert_eq!(spawned(&mut app), Some(vec!["tool03".into()]));
    app.desktop_action_finished(Err("No such file".into()));
    assert!(!app.should_close());
    assert_eq!(app.error.as_deref(), Some("No such file"));
}

#[test]
fn dock_keeps_pins_in_order_and_adds_only_unpinned_apps_from_the_active_workspace() {
    let app = dock_app();
    assert!(
        app.scene
            .bounds(ElementId(DOCK_ELEMENT_ID_BASE + 1))
            .is_some()
    );
    assert!(
        app.scene
            .bounds(ElementId(DOCK_ELEMENT_ID_BASE + 2))
            .is_some()
    );
    assert!(
        app.scene
            .bounds(ElementId(DOCK_ELEMENT_ID_BASE + 3))
            .is_some()
    );
    assert!(
        app.scene
            .bounds(ElementId(DOCK_ELEMENT_ID_BASE + 4))
            .is_none()
    );
    assert!(matches!(
        app.targets.get(&ElementId(DOCK_ELEMENT_ID_BASE + 1)),
        Some(Target::App(index))
            if matches!(&app.apps, Apps::Ready(catalog) if catalog.get(*index).is_some_and(|app| app.id == "tool01.desktop"))
    ));
    assert!(matches!(
        app.targets.get(&ElementId(DOCK_ELEMENT_ID_BASE + 2)),
        Some(Target::DockWindow(WindowId(1)))
    ));
    assert!(matches!(
        app.targets.get(&ElementId(DOCK_ELEMENT_ID_BASE + 3)),
        Some(Target::DockWindow(WindowId(15)))
    ));
}

#[test]
fn dock_launches_idle_pins() {
    let mut app = dock_app();
    click(&mut app, ElementId(DOCK_ELEMENT_ID_BASE + 1));
    assert_eq!(spawned(&mut app), Some(vec!["tool01".into()]));
}

#[test]
fn dock_restores_running_minimized_apps() {
    let mut app = dock_app();
    click(&mut app, ElementId(DOCK_ELEMENT_ID_BASE + 3));
    assert!(matches!(
        app.take_request(),
        Some(HostRequest::Desktop(DesktopCommand::RestoreWindow {
            window: WindowId(15)
        }))
    ));
}

#[test]
fn results_are_applications_only_and_paginate() {
    {
        let mut app = app();
        // Window titles and app IDs from the snapshot must not appear as results.
        type_query(&mut app, "Window");
        assert!(app.search().is_empty());
    }
    // Use a fresh instance for a separate query, independently of visibility actions.
    let mut paginated = app();
    type_query(&mut paginated, "tool");
    assert_eq!(paginated.search().len(), 12);
    let size = paginated.search_page_size();
    assert_eq!(size, 8);
    let first_page: Vec<_> = paginated.search()[..size].to_vec();
    assert!(
        first_page
            .iter()
            .all(|&i| paginated.scene.bounds(app_element(i)).is_some())
    );
    click(&mut paginated, ElementId(31));
    paginated.frame([1200.0, 800.0], &mut Metrics);
    assert!(
        first_page
            .iter()
            .all(|&i| paginated.scene.bounds(app_element(i)).is_none())
    );
    assert!(
        paginated
            .scene
            .bounds(app_element(paginated.search()[size]))
            .is_some()
    );
}

#[test]
fn search_results_leave_room_for_the_overview_dock() {
    let mut compact = app();
    type_query(&mut compact, "tool");
    compact.frame([1200.0, 700.0], &mut Metrics);

    let result_panel = compact.scene.bounds(ElementId(40)).unwrap();
    let dock = compact
        .scene
        .bounds(ElementId(DOCK_ELEMENT_ID_BASE))
        .unwrap();
    assert!(
        result_panel.y + result_panel.height <= dock.y,
        "search results end at {}, overlapping dock starting at {}",
        result_panel.y + result_panel.height,
        dock.y
    );
}
#[test]
fn disconnected_overview_clears_search_without_launching() {
    let mut app = app();
    type_query(&mut app, "edit");
    app.desktop_unavailable();
    key(&mut app, Key::Enter, true);
    key(&mut app, Key::Enter, false);
    assert!(app.take_request().is_none());
    assert!(app.query.text().is_empty());
    assert!(
        app.error
            .as_deref()
            .unwrap()
            .starts_with("Desktop connection unavailable")
    );
}
#[test]
fn queries_without_matches_render_and_a_missing_catalog_is_reported() {
    let mut app = app();
    type_query(&mut app, "zzzz");
    assert!(app.search().is_empty());
    assert!(app.error.is_none());
    key(&mut app, Key::Enter, true);
    assert!(app.take_request().is_none());
    let mut broken = Overview::new();
    broken.apps = Apps::Unavailable;
    broken.desktop_snapshot(&snapshot());
    broken.frame([1200.0, 800.0], &mut Metrics);
    type_query(&mut broken, "edit");
    assert!(broken.search().is_empty());
    assert!(broken.error.is_none());
}

#[test]
fn enabling_all_apps_clears_an_existing_query() {
    let mut app = Overview::new()
        .with_catalog(catalog())
        .with_icon_lookup(icon_lookup(false));
    app.desktop_snapshot(&snapshot());
    app.frame([1200.0, 800.0], &mut Metrics);
    type_query(&mut app, "edit");
    assert_eq!(app.search().len(), 1);

    click(&mut app, ElementId(4));
    assert!(app.all_apps);
    assert!(app.query.text().is_empty());
    assert_eq!(app.search().len(), 13);
}
#[test]
fn enter_right_after_typing_launches_without_waiting_for_a_frame() {
    let mut app = app();
    app.input(Input::Text("edit".into()));
    key(&mut app, Key::Enter, true);
    assert_eq!(
        spawned(&mut app),
        Some(vec!["editor".into(), "--new".into()])
    );
}

fn icon_element(index: usize) -> ElementId {
    ElementId(0x400_0000_0000 + index as u64)
}
#[test]
fn rows_show_an_icon_and_keep_the_text_column_aligned() {
    let mut app = app();
    type_query(&mut app, "edit");
    let index = app.search()[0];
    let row = app.scene.bounds(app_element(index)).unwrap();
    let icon = app.scene.bounds(icon_element(index)).expect("icon element");
    assert_eq!((icon.width, icon.height), (32.0, 32.0));
    assert!(icon.x >= row.x && icon.x + icon.width <= row.x + row.width);
    assert!(icon.y >= row.y && icon.y + icon.height <= row.y + row.height);
    // Clicking the icon activates its row.
    let p = [icon.x + 16.0, icon.y + 16.0];
    app.input(Input::PointerDown(p));
    app.input(Input::PointerUp(p));
    assert_eq!(
        spawned(&mut app),
        Some(vec!["editor".into(), "--new".into()])
    );
}
#[test]
fn icons_are_delivered_by_the_worker_for_visible_rows_only() {
    let mut app = app();
    type_query(&mut app, "tool");
    let ready = |app: &Overview, i: usize| app.scene.bounds(icon_element(i)).is_some();
    let results = app.search();
    assert!(results[..8].iter().all(|&i| ready(&app, i)));
    // Rows on later pages were never requested.
    assert!(results[8..].iter().all(|&i| !ready(&app, i)));
    assert!(!app.is_loading());
    assert!(!app.needs_frame());
}
#[test]
fn enter_while_the_catalog_loads_launches_when_it_arrives() {
    let mut app = Overview::new()
        .with_catalog_environment(apps_environment())
        .with_icon_lookup(icon_lookup(true));
    // Hold the loading state until the deferred action is asserted.
    app.apps = Apps::Loading;
    app.desktop_snapshot(&snapshot());
    app.frame([1200.0, 800.0], &mut Metrics);
    app.input(Input::Text("edit".into()));
    // Enter must not be lost while catalog loading is pending.
    key(&mut app, Key::Enter, true);
    assert!(app.take_request().is_none());
    assert!(app.launch_when_ready);
    app.apps = Apps::Unloaded;
    app.ensure_catalog();
    settle(&mut app, [1200.0, 800.0]);
    assert_eq!(
        spawned(&mut app),
        Some(vec!["editor".into(), "--new".into()])
    );
    assert!(!app.launch_when_ready);
    // Editing the query cancels a pending launch.
    let mut app = Overview::new()
        .with_catalog_environment(apps_environment())
        .with_icon_lookup(icon_lookup(true));
    // Hold the loading state until the deferred action is asserted.
    app.apps = Apps::Loading;
    app.desktop_snapshot(&snapshot());
    app.frame([1200.0, 800.0], &mut Metrics);
    app.input(Input::Text("edit".into()));
    key(&mut app, Key::Enter, true);
    app.input(Input::Text("x".into()));
    app.apps = Apps::Unloaded;
    app.ensure_catalog();
    settle(&mut app, [1200.0, 800.0]);
    assert!(app.take_request().is_none());
}
#[test]
fn a_missing_catalog_directory_is_reported_by_the_worker() {
    let mut app = Overview::new().with_catalog_environment(knave_apps::Environment {
        data_dirs: vec![fixture_dir("empty").join("nowhere")],
        ..Default::default()
    });
    app.desktop_snapshot(&snapshot());
    app.frame([1200.0, 800.0], &mut Metrics);
    type_query(&mut app, "edit");
    assert!(matches!(app.apps, Apps::Unavailable));
    assert!(app.search().is_empty());
}
#[test]
fn missing_icons_use_the_fallback_and_never_fail_the_row() {
    let mut app = app();
    type_query(&mut app, "tool 12");
    let index = app.search()[0];
    assert!(app.scene.bounds(icon_element(index)).is_some());
    // The worker answers a missing name with the theme fallback.
    assert!(app.icons.get("ghost").is_some());
    let mut bare = Overview::new()
        .with_catalog(catalog())
        .with_icon_lookup(icon_lookup(false));
    bare.desktop_snapshot(&snapshot());
    bare.frame([1200.0, 800.0], &mut Metrics);
    type_query(&mut bare, "tool 12");
    let index = bare.search()[0];
    assert!(bare.scene.bounds(app_element(index)).is_some());
    assert!(bare.scene.bounds(icon_element(index)).is_none());
    assert!(bare.error.is_none());
    assert!(!bare.needs_frame());
}
#[test]
fn the_icon_cache_is_bounded_and_requests_are_deduplicated() {
    let mut icons = icons::Icons::default();
    assert!(icons.needs_request("a"));
    icons.requested("a");
    assert!(!icons.needs_request("a"));
    assert!(icons.is_loading());
    for n in 0..400 {
        icons.finish(format!("missing-{n}"), None);
        assert!(icons.len() <= 128);
    }
    icons.finish("a".into(), None);
    assert!(!icons.is_loading());
    assert!(!icons.needs_request("a"));
    assert_eq!(icons.len(), 128);
}
#[test]
fn icons_are_painted_unclipped_not_just_laid_out() {
    let mut app = app();
    type_query(&mut app, "edit");
    let list = app.frame([1200.0, 800.0], &mut Metrics).clone();
    let row_icon = app.scene.bounds(icon_element(app.search()[0])).unwrap();
    let images: Vec<_> = list
        .commands
        .iter()
        .filter_map(|c| match c {
            knave_ui::DisplayCommand::Image { bounds, clip, .. } => Some((*bounds, *clip)),
            _ => None,
        })
        .filter(|(bounds, _)| *bounds == row_icon)
        .collect();
    assert_eq!(images.len(), 1);
    let (bounds, clip) = images[0];
    // A clip that excludes the image would draw nothing while layout still passed.
    let clip = clip.expect("row clips its children");
    assert!(
        bounds.x >= clip.x
            && bounds.y >= clip.y
            && bounds.x + bounds.width <= clip.x + clip.width
            && bounds.y + bounds.height <= clip.y + clip.height,
        "{bounds:?} outside {clip:?}"
    );
}

#[test]
fn launching_then_reopening_resets_the_search_without_reloading_apps() {
    let mut app = app();
    type_query(&mut app, "edit");
    key(&mut app, Key::Enter, true);
    assert!(spawned(&mut app).is_some());
    app.desktop_action_finished(Ok(()));
    assert!(app.take_request().is_some());
    let mut hidden = snapshot();
    hidden.overview_visible = false;
    app.desktop_snapshot(&hidden);
    app.desktop_action_finished(Ok(()));
    // Input queued for the old widget must not resurrect its text.
    key(&mut app, Key::Enter, false);
    app.input(Input::FocusLost);
    app.desktop_snapshot(&snapshot());
    app.input(Input::FocusGained);
    app.frame([1200.0, 800.0], &mut Metrics);
    assert!(app.query.text().is_empty());
    assert!(
        matches!(app.scene.widget(SEARCH), Some(Widget::TextInput(edit)) if edit.text().is_empty())
    );
    assert!(matches!(app.apps, Apps::Ready(_)));
    assert_eq!(app.workspace, Some(WorkspaceId(2)));
    assert!(!app.panes.is_empty());
    assert!(!app.should_close());
    assert!(app.take_request().is_none());
}

#[test]
fn external_hide_resets_browsing_pages_selection_and_errors() {
    let mut app = app();
    publish_workspace(&mut app, WorkspaceId(3));
    type_query(&mut app, "tool");
    app.page = 2;
    app.result_page = 1;
    app.error = Some("Earlier launch failed".into());
    app.background_pressed = true;
    app.pointer = Some([10.0, 10.0]);
    let mut hidden = snapshot();
    hidden.overview_visible = false;
    app.desktop_snapshot(&hidden);
    assert!(app.query.text().is_empty());
    assert_eq!((app.page, app.result_page), (0, 0));
    assert!(app.scene.focus().is_none());
    assert!(app.targets.is_empty());
    assert!(app.error.is_none());
    assert!(!app.background_pressed);
    assert!(app.pointer.is_none());
    let mut reopened = snapshot();
    reopened.workspaces[1].active = false;
    reopened.workspaces[0].active = true;
    app.desktop_snapshot(&reopened);
    app.frame([1200.0, 800.0], &mut Metrics);
    assert_eq!(app.workspace, Some(WorkspaceId(1)));
    assert_eq!((app.page, app.result_page), (0, 0));
    assert_ne!(app.scene.focus(), Some(SEARCH));
}

#[test]
fn hiding_while_catalog_loads_cancels_the_deferred_launch() {
    let mut app = Overview::new()
        .with_catalog_environment(apps_environment())
        .with_icon_lookup(icon_lookup(true));
    app.apps = Apps::Loading;
    app.desktop_snapshot(&snapshot());
    app.frame([1200.0, 800.0], &mut Metrics);
    app.input(Input::Text("edit".into()));
    key(&mut app, Key::Enter, true);
    assert!(app.launch_when_ready);
    let mut hidden = snapshot();
    hidden.overview_visible = false;
    app.desktop_snapshot(&hidden);
    assert!(!app.launch_when_ready);
    app.desktop_snapshot(&snapshot());
    app.apps = Apps::Unloaded;
    app.ensure_catalog();
    settle(&mut app, [1200.0, 800.0]);
    assert!(app.query.text().is_empty());
    assert!(app.take_request().is_none());
    assert!(matches!(app.apps, Apps::Ready(_)));
}

#[test]
fn live_snapshots_preserve_the_query_until_the_view_is_hidden() {
    let mut app = app();
    type_query(&mut app, "edit");
    let mut next = snapshot();
    next.generation += 1;
    next.windows[0].title = "New title".into();
    app.desktop_snapshot(&next);
    app.frame([1200.0, 800.0], &mut Metrics);
    assert_eq!(app.query.text(), "edit");
    app.desktop_unavailable();
    assert!(app.query.text().is_empty());
    assert!(!app.surface_visible());
    app.desktop_snapshot(&next);
    app.frame([1200.0, 800.0], &mut Metrics);
    assert!(app.query.text().is_empty());
}

#[test]
fn overview_backdrop_preserves_wallpaper_visibility() {
    let mut app = app();
    let list = app.frame([1200.0, 800.0], &mut Metrics);
    assert_eq!(list.clear_color, knave_ui::Color::rgba(0, 0, 0, 0));
    assert!(
        !list.commands.iter().any(|command| matches!(command,
            knave_ui::DisplayCommand::Shape { bounds, .. }
                if bounds.width == 1200.0 && bounds.height == 800.0
        )),
        "a structural fullscreen root must not emit a surface shape"
    );
}

#[test]
fn overview_controls_follow_the_selected_toolkit_theme() {
    let mut app = Overview::with_theme(knave_ui::toolkit::Theme::Light);
    app.desktop_snapshot(&snapshot());
    app.frame([1200.0, 800.0], &mut Metrics);

    let search = app.scene.computed_style(SEARCH).unwrap();
    let search_paint = search.style.background.as_ref().unwrap();
    let mut defaults = Scene::new(Element::new(
        1,
        Widget::TextInput(TextEdit::new("").unwrap()),
    ))
    .unwrap();
    defaults.set_theme(knave_ui::toolkit::Theme::Light).unwrap();
    let baseline = defaults.computed_style(ElementId(1)).unwrap();
    assert_eq!(
        search_paint.fill,
        baseline.style.background.and_then(|paint| paint.fill)
    );
    assert_eq!(search.outline.map(|outline| outline.width), Some(1.0));
    let selected_card = app
        .scene
        .computed_style(ElementId(0x100_0000_0000 + 2 * 32))
        .unwrap();
    assert!(
        selected_card
            .outline
            .is_none_or(|outline| outline.width == 0.0)
    );
}

#[test]
fn overview_uses_preview_and_arrows_without_redundant_workspace_dots() {
    let app = app();
    assert!(app.scene.bounds(ElementId(21)).is_some());
    assert!(
        app.scene
            .bounds(ElementId(0x100_0000_0000 + 3 * 32))
            .is_some()
    );
    for workspace in 1..=3 {
        assert!(
            app.scene
                .bounds(ElementId(0x200_0000_0000 + workspace))
                .is_none()
        );
    }
}

#[test]
fn workspace_panes_fill_cards_without_reserved_empty_shelf() {
    let mut app = app();
    app.frame([1280.0, 800.0], &mut Metrics);
    for pane in &app.panes {
        let id = ElementId(0x100_0000_0000 + u64::from(pane.workspace.0) * 32);
        let card = app.scene.bounds(id).unwrap();
        assert_eq!(pane.width, (card.width - 4.0).round() as u32);
        assert_eq!(pane.height, (card.height - 4.0).round() as u32);
        assert_eq!(pane.x, (card.x + 2.0).round() as i32);
        assert_eq!(pane.y, (card.y + 2.0).round() as i32);
    }
}

/// Retained controls follow the shared bar's hit-target order.
fn bar_element(node_id: u64) -> ElementId {
    ElementId(0x800_0000_0000 + node_id)
}
fn bar_target(app: &Overview, action: knave_shell_ui::UiAction) -> ElementId {
    *app.targets
        .iter()
        .find_map(|(id, target)| {
            matches!(target, Target::StatusBar(candidate) if *candidate == action).then_some(id)
        })
        .expect("shared bar action exists")
}
fn status_title(app: &Overview) -> Option<String> {
    let bar = app.status_bar_component(app.size[0]);
    let (title_bounds, title) = bar.scene.nodes().iter().find_map(|node| match node {
        UiNode::Label {
            id, bounds, text, ..
        } if id.0 == 8 => Some((*bounds, text)),
        _ => None,
    })?;
    app.display
        .commands
        .iter()
        .find_map(|command| match command {
            knave_ui::DisplayCommand::Text { bounds, text, .. }
                if *bounds == title_bounds && text == title =>
            {
                Some(text.clone())
            }
            _ => None,
        })
}

#[test]
fn the_overview_routes_the_desktop_status_bar_targets() {
    let mut app = app();
    app.frame([1200.0, 800.0], &mut Metrics);
    // The bar component renders the active workspace, exactly as outside overview.
    assert_eq!(status_title(&app).as_deref(), Some("Window 1"));
    assert!(app.targets.values().any(|target| matches!(
        target,
        Target::StatusBar(knave_shell_ui::UiAction::OpenOverview)
    )));
    let shared = app.status_bar_component(1200.0);
    let chips: Vec<_> = shared
        .scene
        .hit_targets()
        .filter_map(|(bounds, action)| {
            if let knave_shell_ui::UiAction::FocusWorkspace(workspace) = action {
                Some((bounds, workspace))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(chips.len(), 3);
    assert!(
        chips[0].0.x < 80.0,
        "no legacy reserved gap before workspaces"
    );
    for (bounds, workspace) in chips {
        assert!(bounds.width <= 26.0, "compact workspace chip");
        assert_eq!(
            app.scene.bounds(bar_target(
                &app,
                knave_shell_ui::UiAction::FocusWorkspace(workspace)
            )),
            Some(bounds)
        );
    }
    // A chip click switches that workspace directly, like the desktop bar.
    let third = bar_target(
        &app,
        knave_shell_ui::UiAction::FocusWorkspace(WorkspaceId(3)),
    );
    click(&mut app, third);
    assert!(matches!(
        app.take_request(),
        Some(HostRequest::Desktop(DesktopCommand::FocusWorkspace {
            workspace: WorkspaceId(3)
        }))
    ));
    app.desktop_action_finished(Ok(()));
    assert!(app.take_request().is_none());
    assert!(app.surface_visible());
}

#[test]
fn overview_rejects_stale_focused_icons_and_clears_on_disconnect() {
    let mut app = app();
    let current = knave_ui::UiImage::from_rgba(1, 1, vec![1, 2, 3, 255]).unwrap();
    let stale = knave_ui::UiImage::from_rgba(1, 1, vec![4, 5, 6, 255]).unwrap();
    app.focused_app_icon(Some("Editor"), Some(&current));
    assert_eq!(app.focused_icon.as_ref(), Some(&current));
    app.focused_app_icon(Some("old.application"), Some(&stale));
    assert_eq!(app.focused_icon.as_ref(), Some(&current));
    app.desktop_unavailable();
    assert!(app.focused_icon.is_none());
}

#[test]
fn the_shared_bar_clears_the_title_when_the_focused_window_is_on_another_workspace() {
    let mut app = app();
    publish_workspace(&mut app, WorkspaceId(3));
    app.frame([1200.0, 800.0], &mut Metrics);
    assert_eq!(status_title(&app), None);
    // The bar, including its chips, stays visible while searching.
    type_query(&mut app, "edit");
    assert!(
        app.scene
            .bounds(bar_target(
                &app,
                knave_shell_ui::UiAction::FocusWorkspace(WorkspaceId(3))
            ))
            .is_some()
    );
    let search = app.scene.bounds(SEARCH).unwrap();
    assert!(search.y >= 36.0, "search overlaps the status bar");
}

#[test]
fn shared_bar_text_matches_the_desktop_bar_display_list() {
    let mut app = app();
    let list = app.frame([1200.0, 800.0], &mut Metrics).clone();
    let bar = knave_shell_ui::status_bar::StatusBar::with_focused_icon(
        0,
        1200.0,
        36.0,
        app.snapshot(),
        true,
        None,
        &[],
        app.focused_icon.as_ref(),
    );
    for node in bar.scene.nodes().iter() {
        let (bounds, text) = match node {
            UiNode::Label { bounds, text, .. } => (*bounds, text.clone()),
            _ => continue,
        };
        let projected = list.commands.iter().any(|command| match command {
            knave_ui::DisplayCommand::Text {
                bounds: rendered_bounds,
                text: candidate,
                ..
            } => candidate == &text && *rendered_bounds == bounds,
            _ => false,
        });
        assert!(projected, "label {text:?} keeps its widget allocation");
    }
}

#[test]
fn the_shared_bar_strip_is_rendered_by_the_statusbar_widgets() {
    let mut app = app();
    let list = app.frame([1200.0, 800.0], &mut Metrics).clone();
    // The overview keeps hit-test widgets under the shared bar display list.
    let strip: Vec<_> = list
        .commands
        .iter()
        .filter_map(|command| match command {
            knave_ui::DisplayCommand::Shape { bounds, paint, .. }
                if bounds.y <= 0.0 && bounds.height <= 40.0 && bounds.width >= 1100.0 =>
            {
                Some(paint.fill)
            }
            _ => None,
        })
        .collect();
    let shared = app.status_bar_component(1200.0);
    let background = match &shared.display.commands[0] {
        knave_ui::DisplayCommand::Shape { paint, .. } => paint.fill,
        _ => panic!("bar background is the first command"),
    };
    assert_eq!(
        strip,
        vec![background],
        "only the real status bar paints the strip; its input container does not"
    );
}

#[test]
fn without_a_snapshot_the_shared_bar_shows_the_brand_mark_without_a_wordmark() {
    let mut app = Overview::new();
    app.frame([1200.0, 800.0], &mut Metrics);
    // Icon-only mark: no disconnected "Knave" title is painted beside it.
    assert!(status_title(&app).as_deref().is_none_or(str::is_empty));
    let brand_bounds = app
        .status_bar_component(1200.0)
        .scene
        .nodes()
        .iter()
        .find_map(|node| match node {
            UiNode::Image { id, bounds, .. } if id.0 == 3 => Some(*bounds),
            _ => None,
        })
        .expect("brand image in shared component");
    let image = app.display.commands.iter().any(|command| {
        matches!(command,
        knave_ui::DisplayCommand::Image { bounds, .. } if *bounds == brand_bounds)
    });
    assert!(image, "the bundled brand mark renders on the shared bar");
    assert!(!app.targets.values().any(|target| matches!(
        target,
        Target::StatusBar(knave_shell_ui::UiAction::FocusWorkspace(_))
    )));
}

#[test]
fn the_brand_mark_toggles_the_visible_overview_off() {
    let mut app = app();
    app.frame([1200.0, 800.0], &mut Metrics);
    let id = *app
        .targets
        .iter()
        .find(|(_, target)| {
            matches!(
                target,
                Target::StatusBar(knave_shell_ui::UiAction::OpenOverview)
            )
        })
        .unwrap()
        .0;
    click(&mut app, id);
    assert!(matches!(
        app.take_request(),
        Some(HostRequest::Desktop(DesktopCommand::SetOverviewVisible {
            visible: false
        }))
    ));
}

#[test]
fn overview_renders_the_complete_shared_bar_and_routes_its_bell() {
    let mut app = app();
    app.frame([1200.0, 800.0], &mut Metrics);
    let shared = app.status_bar_component(1200.0);
    assert!(app.display.commands.ends_with(&shared.display.commands));
    let (index, _) = shared
        .scene
        .hit_targets()
        .enumerate()
        .find(|(_, (_, action))| *action == knave_shell_ui::UiAction::ToggleNotifications)
        .unwrap();
    click(&mut app, bar_element(10 + index as u64));
    assert!(matches!(
        app.take_request(),
        Some(HostRequest::Desktop(DesktopCommand::SetOverviewVisible {
            visible: false
        }))
    ));
    app.desktop_action_finished(Ok(()));
    assert!(matches!(
        app.take_request(),
        Some(HostRequest::ToggleNotifications)
    ));
    app.notifications_enabled(false);
    app.frame([1200.0, 800.0], &mut Metrics);
    assert!(
        !app.status_bar_component(1200.0)
            .scene
            .hit_targets()
            .any(|(_, action)| action == knave_shell_ui::UiAction::ToggleNotifications)
    );
    assert!(!app.targets.values().any(|target| matches!(
        target,
        Target::StatusBar(knave_shell_ui::UiAction::ToggleNotifications)
    )));
    assert!(app.targets.values().any(|target| matches!(
        target,
        Target::StatusBar(knave_shell_ui::UiAction::ToggleQuickSettings)
    )));
}

#[test]
fn overview_tracks_shared_notification_indicators_on_updates_and_disconnect() {
    use knave_desktop_api::notifications::{Notification, NotificationSnapshot};
    let mut app = app();
    let mut state = NotificationSnapshot {
        owned: true,
        dnd: true,
        ..Default::default()
    };
    state.notifications = vec![
        Notification {
            unread: true,
            ..Default::default()
        };
        100
    ];
    app.notification_snapshot(Some(&state));
    app.frame([1200.0, 800.0], &mut Metrics);
    let shared = knave_shell_ui::status_bar::StatusBar::with_focused_icon(
        0,
        1200.0,
        36.0,
        app.snapshot(),
        true,
        Some(&state),
        &[],
        app.focused_icon.as_ref(),
    );
    assert!(app.display.commands.ends_with(&shared.display.commands));
    for expected in ["99+", "z"] {
        assert!(app.display.commands.iter().any(|command| matches!(command,
            knave_ui::DisplayCommand::Text {text, ..} if text == expected)));
    }
    app.notification_snapshot(None);
    app.frame([1200.0, 800.0], &mut Metrics);
    let shared = knave_shell_ui::status_bar::StatusBar::with_focused_icon(
        0,
        1200.0,
        36.0,
        app.snapshot(),
        true,
        None,
        &[],
        app.focused_icon.as_ref(),
    );
    assert!(app.display.commands.ends_with(&shared.display.commands));
    assert!(!app.display.commands.iter().any(|command| matches!(command,
        knave_ui::DisplayCommand::Text {text, ..} if text == "99+" || text == "z")));
}

#[test]
fn failed_overview_hide_does_not_open_notifications_behind_it() {
    let mut app = app();
    app.frame([1200.0, 800.0], &mut Metrics);
    let bell = app
        .targets
        .iter()
        .find_map(|(id, target)| {
            matches!(
                target,
                Target::StatusBar(knave_shell_ui::UiAction::ToggleNotifications)
            )
            .then_some(*id)
        })
        .unwrap();
    click(&mut app, bell);
    app.take_request();
    app.desktop_action_finished(Err("hide failed".into()));
    assert!(app.take_request().is_none());
    assert_eq!(app.error.as_deref(), Some("hide failed"));
}

#[test]
fn quick_settings_hides_overview_before_transferring_focus() {
    let mut app = app();
    app.frame([1200.0, 800.0], &mut Metrics);
    let id = *app
        .targets
        .iter()
        .find(|(_, target)| {
            matches!(
                target,
                Target::StatusBar(knave_shell_ui::UiAction::ToggleQuickSettings)
            )
        })
        .unwrap()
        .0;
    click(&mut app, id);
    assert!(matches!(
        app.take_request(),
        Some(HostRequest::Desktop(DesktopCommand::SetOverviewVisible {
            visible: false
        }))
    ));
    app.desktop_action_finished(Ok(()));
    assert!(matches!(
        app.take_request(),
        Some(HostRequest::ToggleQuickSettings)
    ));
}

#[test]
fn the_workspace_stage_keeps_a_compact_band_under_the_strip() {
    let mut app = app();
    app.frame([1200.0, 800.0], &mut Metrics);
    // The reduced stage margins leave more room for the preview.
    let search = app.scene.bounds(SEARCH).unwrap();
    let card = app
        .scene
        .bounds(ElementId(0x100_0000_0000 + 2 * 32))
        .unwrap();
    assert!(search.y >= STATUS_BAR_HEIGHT);
    assert!(card.height >= 550.0);
    // The overview no longer renders its redundant workspace dot row.
    assert!(app.scene.bounds(ElementId(0x200_0000_0000 + 2)).is_none());
}

#[test]
fn full_named_themes_leave_compositor_preview_windows_unpainted() {
    for theme in [
        knave_ui::toolkit::Theme::Dark,
        knave_ui::toolkit::Theme::Light,
    ] {
        let mut app = app();
        app.theme_changed(theme, theme.stylesheet()).unwrap();
        let list = app.frame([1200.0, 800.0], &mut Metrics).clone();
        let pane = app
            .panes
            .iter()
            .find(|pane| pane.workspace == WorkspaceId(2))
            .unwrap();
        let x = pane.x as f32 + pane.width as f32 / 2.0;
        let y = pane.y as f32 + pane.height as f32 / 2.0;
        assert!(
            !list.commands.iter().any(|command| matches!(command,
                knave_ui::DisplayCommand::Shape { bounds, paint, .. }
                    if bounds.x <= x && bounds.y <= y
                        && bounds.x + bounds.width > x && bounds.y + bounds.height > y
                        && paint.fill.is_some_and(|color| color.alpha > 0)
            )),
            "{theme:?} shell chrome must not obscure compositor-rendered windows"
        );
    }
}
