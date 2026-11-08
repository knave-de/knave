//! Real managed windows exercise the shared navigation and swipe target contract.
use super::*;
use crate::{dispatch::Dispatch as Action, workspace_navigation::Direction};

#[test]
#[ignore = "requires private Unix sockets and an isolated XDG_RUNTIME_DIR"]
fn sparse_navigation_and_preview_lifecycle() {
    let mut event_loop = EventLoop::try_new().unwrap();
    let mut state = Villain::new(
        &mut event_loop,
        Display::new().unwrap(),
        crate::config::RuntimeConfig::load().unwrap(),
    );
    let (server, client) = UnixStream::pair().unwrap();
    state
        .display_handle
        .insert_client(server, Arc::new(crate::state::ClientState::default()))
        .unwrap();
    let (sender, requests) = mpsc::channel::<Inspect>();
    let thread = std::thread::spawn(move || {
        let conn = Connection::from_socket(client).unwrap();
        let (globals, mut queue) = registry_queue_init::<Client>(&conn).unwrap();
        let qh = queue.handle();
        let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=4, ()).unwrap();
        let wm: xdg_wm_base::XdgWmBase = globals.bind(&qh, 1..=6, ()).unwrap();
        let mut client = Client::default();
        let mut windows = Vec::new();
        for (index, workspace) in [0, 3, 6].into_iter().enumerate() {
            inspect(&sender, move |state| state.switch_workspace(workspace));
            let surface = compositor.create_surface(&qh, ());
            let xdg = wm.get_xdg_surface(&surface, &qh, ());
            let top = xdg.get_toplevel(&qh, index);
            top.set_app_id(format!("sparse-{workspace}"));
            surface.commit();
            queue.roundtrip(&mut client).unwrap();
            queue.roundtrip(&mut client).unwrap();
            windows.push((surface, xdg, top));
        }
        inspect(&sender, |state| {
            state.switch_workspace(0);
            state.config.workspace.skip_empty = false;
            state.dispatch(Action::NextWorkspace).unwrap();
            assert_eq!(state.active_workspace, 1, "default visits empty slots");
            state.config.workspace.skip_empty = true;
            state.dispatch(Action::NextWorkspace).unwrap();
            assert_eq!(state.active_workspace, 3, "escape an empty source");
            state.dispatch(Action::NextWorkspace).unwrap();
            assert_eq!(state.active_workspace, 6);
            state.dispatch(Action::NextWorkspace).unwrap();
            assert_eq!(state.active_workspace, 7, "advance to the dynamic tail");
            state.dispatch(Action::PreviousWorkspace).unwrap();
            assert_eq!(state.active_workspace, 6);
            state.dispatch(Action::FocusWorkspace(3)).unwrap();
            assert_eq!(
                state.active_workspace, 2,
                "direct selection reaches empty workspace"
            );
            state.switch_workspace(0);
        });
        windows[1].2.set_minimized();
        queue.roundtrip(&mut client).unwrap();
        inspect(&sender, |state| {
            assert!(find(state, "sparse-3").minimized);
            assert_eq!(
                state.workspace_target(0, Direction::Next),
                Some(3),
                "minimized-only workspace remains occupied"
            );
            state.dispatch(Action::NextWorkspace).unwrap();
            assert_eq!(state.active_workspace, 3);
            assert!(state.restore_window(find(state, "sparse-3").id));
            state.switch_workspace(0);
            state.swipe_begin("pad".into(), 3, 1);
            state.swipe_update("pad", -100.0, 0.0, 2);
            let preview = state.gestures.transition.unwrap();
            assert_eq!((preview.target, preview.direction), (3, Direction::Next));
            // Place a real managed window into an intervening inactive slot without
            // explicitly switching workspaces (which intentionally cancels a swipe).
            let entry = state.workspaces[6].windows.pop().unwrap();
            state.workspaces[1].windows.push(entry);
            assert_eq!(state.workspace_target(0, Direction::Next), Some(1));
            state.swipe_update("pad", -10.0, 0.0, 3);
            assert_eq!(
                state.gestures.transition.unwrap().target,
                3,
                "preview does not retarget to a newly occupied slot"
            );
            state.swipe_end("pad", false, 4);
            assert_eq!(
                state.active_workspace, 3,
                "release commits the previewed target"
            );
            let entry = state.workspaces[1].windows.pop().unwrap();
            state.workspaces[6].windows.push(entry);
            state.switch_workspace(0);
            state.swipe_begin("pad".into(), 3, 5);
            state.swipe_update("pad", -100.0, 0.0, 6);
            state.swipe_update("pad", 210.0, 0.0, 7);
            let preview = state.gestures.transition.unwrap();
            assert_eq!(
                (preview.target, preview.direction),
                (6, Direction::Previous)
            );
            state.swipe_end("pad", false, 8);
            assert_eq!(
                state.active_workspace, 6,
                "reversal commits the opposite sparse target"
            );
            state.switch_workspace(0);
            state.swipe_begin("pad".into(), 3, 9);
            state.swipe_update("pad", -100.0, 0.0, 10);
        });
        // Destroy the previewed target through the real Wayland lifecycle.
        let (surface, xdg, top) = &windows[1];
        top.destroy();
        xdg.destroy();
        surface.destroy();
        queue.roundtrip(&mut client).unwrap();
        inspect(&sender, |state| {
            state.maintain_swipe();
            assert!(!state.workspace_occupied(3));
            assert!(state.gestures.transition.is_none());
            state.swipe_update("pad", -100.0, 0.0, 11);
            state.swipe_end("pad", false, 12);
            assert_eq!(
                state.active_workspace, 0,
                "target destruction cancels rather than retargets"
            );
        });
        let (surface, xdg, top) = &windows[2];
        top.destroy();
        xdg.destroy();
        surface.destroy();
        queue.roundtrip(&mut client).unwrap();
        inspect(&sender, |state| {
            state.dispatch(Action::NextWorkspace).unwrap();
            state.dispatch(Action::PreviousWorkspace).unwrap();
            state.swipe_begin("pad".into(), 3, 13);
            state.swipe_update("pad", -200.0, 0.0, 14);
            assert!(state.gestures.transition.is_none());
            state.swipe_end("pad", false, 15);
            assert_eq!(
                state.active_workspace, 0,
                "only occupied workspace stays put"
            );
        });
        let (surface, xdg, top) = &windows[0];
        top.destroy();
        xdg.destroy();
        surface.destroy();
        queue.roundtrip(&mut client).unwrap();
        inspect(&sender, |state| {
            assert_eq!(
                state.active_workspace, 0,
                "closing last window never auto-leaves"
            );
            state.dispatch(Action::FocusWorkspace(5)).unwrap();
            assert_eq!(
                state.active_workspace, 0,
                "number shortcut cannot select beyond the materialized tail"
            );
            state.dispatch(Action::NextWorkspace).unwrap();
            state.dispatch(Action::PreviousWorkspace).unwrap();
            assert_eq!(
                state.active_workspace, 0,
                "all empty navigation stays on the first workspace"
            );
        });
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    while !thread.is_finished() {
        assert!(
            Instant::now() < deadline,
            "sparse navigation test timed out"
        );
        event_loop
            .dispatch(Duration::from_millis(2), &mut state)
            .unwrap();
        while let Ok(inspect) = requests.try_recv() {
            inspect(&mut state);
        }
        state.maintain_swipe();
        state.space.refresh();
        state.display_handle.flush_clients().unwrap();
    }
    thread.join().unwrap();
}

#[test]
#[ignore = "requires private Unix sockets and an isolated XDG_RUNTIME_DIR"]
fn last_workspace_tracks_successful_switches_and_toggles() {
    let mut event_loop = EventLoop::try_new().unwrap();
    let mut state = Villain::new(
        &mut event_loop,
        Display::new().unwrap(),
        crate::config::RuntimeConfig::load().unwrap(),
    );
    state.dispatch(Action::LastWorkspace).unwrap();
    assert_eq!(state.active_workspace, 0, "no history at startup");
    assert_eq!(state.last_workspace, None);
    for _ in 0..6 {
        state.dispatch(Action::NextWorkspace).unwrap();
    }
    assert_eq!(state.workspaces.len(), 8);
    state.dispatch(Action::FocusWorkspace(7)).unwrap();
    state.dispatch(Action::FocusWorkspace(7)).unwrap();
    state.switch_workspace(10);
    assert!(state.dispatch(Action::FocusWorkspace(0)).is_err());
    assert_eq!(
        state.last_workspace,
        Some(6),
        "no-op/invalid switches preserve history"
    );
    state.config.workspace.skip_empty = true;
    state.dispatch(Action::LastWorkspace).unwrap();
    assert_eq!(
        state.active_workspace, 6,
        "history includes empty workspaces"
    );
    state.dispatch(Action::LastWorkspace).unwrap();
    assert_eq!(state.active_workspace, 10, "repeated use toggles");
    state.config.workspace.skip_empty = false;
    state.dispatch(Action::PreviousWorkspace).unwrap();
    state.dispatch(Action::LastWorkspace).unwrap();
    assert_eq!(
        state.active_workspace, 10,
        "directional switches update history"
    );
    state.swipe_begin("pad".into(), 3, 1);
    state.swipe_update("pad", -100.0, 0.0, 2);
    state.swipe_end("pad", false, 3);
    assert_eq!(state.active_workspace, 11);
    state.dispatch(Action::LastWorkspace).unwrap();
    assert_eq!(
        state.active_workspace, 10,
        "committed swipes update history"
    );
    state.swipe_begin("pad".into(), 3, 4);
    state.swipe_update("pad", -100.0, 0.0, 5);
    state.swipe_end("pad", true, 6);
    state.dispatch(Action::LastWorkspace).unwrap();
    assert_eq!(
        state.active_workspace, 11,
        "cancelled swipes preserve history"
    );
}

#[test]
#[ignore = "requires private Unix sockets and an isolated XDG_RUNTIME_DIR"]
fn move_workspace_keyboard_preserves_family_and_focus() {
    let mut event_loop = EventLoop::try_new().unwrap();
    let mut state = Villain::new(
        &mut event_loop,
        Display::new().unwrap(),
        crate::config::RuntimeConfig::load().unwrap(),
    );
    let (server, client) = UnixStream::pair().unwrap();
    state
        .display_handle
        .insert_client(server, Arc::new(crate::state::ClientState::default()))
        .unwrap();
    let (sender, requests) = mpsc::channel::<Inspect>();
    let thread = std::thread::spawn(move || {
        let conn = Connection::from_socket(client).unwrap();
        let (globals, mut queue) = registry_queue_init::<Client>(&conn).unwrap();
        let qh = queue.handle();
        let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=4, ()).unwrap();
        let wm: xdg_wm_base::XdgWmBase = globals.bind(&qh, 1..=6, ()).unwrap();
        let mut client = Client::default();
        let mut windows: Vec<(
            wl_surface::WlSurface,
            xdg_surface::XdgSurface,
            xdg_toplevel::XdgToplevel,
        )> = Vec::new();
        for (index, app) in ["remaining", "parent", "dialog"].into_iter().enumerate() {
            let surface = compositor.create_surface(&qh, ());
            let xdg = wm.get_xdg_surface(&surface, &qh, ());
            let top = xdg.get_toplevel(&qh, index);
            top.set_app_id(app.into());
            if index == 2 {
                top.set_parent(Some(&windows[1].2));
            }
            surface.commit();
            queue.roundtrip(&mut client).unwrap();
            queue.roundtrip(&mut client).unwrap();
            windows.push((surface, xdg, top));
        }
        inspect(&sender, |state| {
            use crate::keybinds::handle_keyboard_key;
            use smithay::{
                backend::input::KeyState::{Pressed, Released},
                input::keyboard::Keycode,
            };
            let parent = find(state, "parent").id;
            let dialog = find(state, "dialog").id;
            assert!(state.focus_window(parent).unwrap());
            state.maximize_focused_window(Some(true));
            assert!(state.focus_window(dialog).unwrap());
            let history = state.last_workspace;
            assert!(state.dispatch(Action::MoveFocusedToWorkspace(0)).is_err());
            assert!(state.dispatch(Action::MoveFocusedToWorkspace(11)).is_err());
            state.dispatch(Action::MoveFocusedToWorkspace(1)).unwrap();
            assert!(find(state, "dialog").focused);
            handle_keyboard_key(state, Keycode::new(133), Pressed, 1);
            handle_keyboard_key(state, Keycode::new(50), Pressed, 2);
            handle_keyboard_key(state, Keycode::new(11), Pressed, 3); // 2, with Shift held
            handle_keyboard_key(state, Keycode::new(11), Released, 4);
            handle_keyboard_key(state, Keycode::new(50), Released, 5);
            handle_keyboard_key(state, Keycode::new(133), Released, 6);
            assert_eq!(state.active_workspace, 0);
            assert_eq!(state.last_workspace, history);
            assert!(find(state, "remaining").focused);
            for app in ["parent", "dialog"] {
                assert_eq!(find(state, app).workspace.0, 2);
                assert!(!mapped(state, app));
            }
            assert!(
                state.workspaces[0]
                    .focus_history
                    .iter()
                    .all(|id| *id != parent && *id != dialog)
            );
            assert!(find(state, "parent").maximized);
            assert!(find(state, "dialog").floating);
            state.switch_workspace(1);
            assert!(mapped(state, "parent") && mapped(state, "dialog"));
            assert!(find(state, "dialog").focused);
            state.dispatch(Action::MoveFocusedToWorkspace(10)).unwrap();
            assert!(state.focused_window().is_none());
            assert!(state.workspaces[1].is_empty());
            assert!(state.dispatch(Action::MoveFocusedToWorkspace(3)).is_err());
            assert!(state.focus_window(parent).unwrap());
            assert_eq!(state.active_workspace, 9);
            assert!(mapped(state, "parent"));
            assert!(state.focus_window(dialog).unwrap());
            state.minimize_focused_window();
            assert!(state.focus_window(parent).unwrap());
            let window = state.focused_window().unwrap();
            state.set_window_fullscreen(&window, true);
            state.dispatch(Action::MoveFocusedToWorkspace(1)).unwrap();
            assert!(state.workspaces[9].is_empty());
            assert!(state.workspaces[9].minimized_history.is_empty());
            assert!(find(state, "parent").fullscreen);
            assert!(find(state, "dialog").minimized);
            state.switch_workspace(0);
            assert!(find(state, "parent").focused);
            assert!(mapped(state, "parent"));
            assert!(!mapped(state, "dialog"));
            assert!(state.restore_last_minimized_window());
            assert!(!find(state, "dialog").minimized);
            assert!(mapped(state, "dialog"));
        });
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    while !thread.is_finished() {
        assert!(
            Instant::now() < deadline,
            "sparse navigation test timed out"
        );
        event_loop
            .dispatch(Duration::from_millis(2), &mut state)
            .unwrap();
        while let Ok(inspect) = requests.try_recv() {
            inspect(&mut state);
        }
        state.maintain_swipe();
        state.space.refresh();
        state.display_handle.flush_clients().unwrap();
    }
    thread.join().unwrap();
}
