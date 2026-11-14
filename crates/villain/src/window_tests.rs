//! Real Wayland protocol requests against a private, headless Villain instance.
use super::*;
use smithay::reexports::{calloop::EventLoop, wayland_server::Display};
use std::{
    collections::HashMap,
    os::{fd::AsFd, unix::net::UnixStream},
    path::PathBuf,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};
use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    globals::{GlobalList, GlobalListContents, registry_queue_init},
    protocol::{
        wl_buffer, wl_compositor, wl_keyboard, wl_pointer, wl_registry, wl_seat, wl_shm,
        wl_shm_pool, wl_surface,
    },
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

#[derive(Default, Clone, Debug)]
struct Configure {
    width: i32,
    height: i32,
    fullscreen: bool,
    maximized: bool,
    tiled: bool,
}
#[derive(Default)]
struct Client {
    configures: HashMap<usize, Configure>,
    configure_counts: HashMap<usize, usize>,
    decoration_configure_counts: HashMap<usize, usize>,
    pointer_buttons: usize,
    swipes: Vec<String>,
    pinches: Vec<String>,
    holds: Vec<String>,
    keyboard_modifiers: Vec<u32>,
    keyboard_keys: Vec<u32>,
    keyboard_enters: Vec<(wl_surface::WlSurface, Vec<u8>)>,
    inhibitor_states: Vec<bool>,
    closes: usize,
    decoration_modes: HashMap<usize, u32>,
}
impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Client {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Client {
    fn event(
        _: &mut Self,
        base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            base.pong(serial);
        }
    }
}
impl Dispatch<xdg_surface::XdgSurface, ()> for Client {
    fn event(
        _: &mut Self,
        surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            surface.ack_configure(serial);
        }
    }
}
impl Dispatch<xdg_toplevel::XdgToplevel, usize> for Client {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        id: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Configure {
            width,
            height,
            states,
        } = event
        {
            let states: Vec<_> = states
                .as_chunks::<4>()
                .0
                .iter()
                .map(|bytes| u32::from_ne_bytes(*bytes))
                .collect();
            state.configures.insert(
                *id,
                Configure {
                    width,
                    height,
                    maximized: states.contains(&(xdg_toplevel::State::Maximized as u32)),
                    fullscreen: states.contains(&(xdg_toplevel::State::Fullscreen as u32)),
                    tiled: states.contains(&(xdg_toplevel::State::TiledLeft as u32)),
                },
            );
            *state.configure_counts.entry(*id).or_default() += 1;
        } else if let xdg_toplevel::Event::Close = event {
            state.closes += 1;
        }
    }
}
impl Dispatch<wl_pointer::WlPointer, ()> for Client {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_pointer::Event::Button { .. }) {
            state.pointer_buttons += 1;
        }
    }
}
use wayland_protocols::wp::pointer_gestures::zv1::client::{
    zwp_pointer_gesture_hold_v1, zwp_pointer_gesture_pinch_v1, zwp_pointer_gesture_swipe_v1,
    zwp_pointer_gestures_v1,
};
delegate_noop!(Client: ignore zwp_pointer_gestures_v1::ZwpPointerGesturesV1);
impl Dispatch<zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1, ()> for Client {
    fn event(
        state: &mut Self,
        _: &zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1,
        event: zwp_pointer_gesture_pinch_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_pointer_gesture_pinch_v1::Event;
        state.pinches.push(
            match event {
                Event::Begin { .. } => "begin",
                Event::Update { .. } => "update",
                Event::End { cancelled: 0, .. } => "end",
                Event::End { .. } => "cancel",
                _ => "other",
            }
            .into(),
        );
    }
}
impl Dispatch<zwp_pointer_gesture_hold_v1::ZwpPointerGestureHoldV1, ()> for Client {
    fn event(
        state: &mut Self,
        _: &zwp_pointer_gesture_hold_v1::ZwpPointerGestureHoldV1,
        event: zwp_pointer_gesture_hold_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_pointer_gesture_hold_v1::Event;
        state.holds.push(
            match event {
                Event::Begin { .. } => "begin",
                Event::End { cancelled: 0, .. } => "end",
                Event::End { .. } => "cancel",
                _ => "other",
            }
            .into(),
        );
    }
}
impl Dispatch<zwp_pointer_gesture_swipe_v1::ZwpPointerGestureSwipeV1, ()> for Client {
    fn event(
        state: &mut Self,
        _: &zwp_pointer_gesture_swipe_v1::ZwpPointerGestureSwipeV1,
        event: zwp_pointer_gesture_swipe_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_pointer_gesture_swipe_v1::Event;
        state.swipes.push(
            match event {
                Event::Begin { .. } => "begin",
                Event::Update { .. } => "update",
                Event::End { cancelled: 0, .. } => "end",
                Event::End { .. } => "cancel",
                _ => "other",
            }
            .into(),
        );
    }
}
impl Dispatch<wl_keyboard::WlKeyboard, ()> for Client {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Modifiers { mods_depressed, .. } => {
                state.keyboard_modifiers.push(mods_depressed)
            }
            wl_keyboard::Event::Key { key, .. } => state.keyboard_keys.push(key),
            wl_keyboard::Event::Enter { surface, keys, .. } => {
                state.keyboard_enters.push((surface, keys))
            }
            _ => {}
        }
    }
}
delegate_noop!(Client: ignore wayland_client::protocol::wl_callback::WlCallback);
delegate_noop!(Client: ignore wl_shm::WlShm);
delegate_noop!(Client: ignore wl_shm_pool::WlShmPool);
delegate_noop!(Client: ignore wl_buffer::WlBuffer);
delegate_noop!(Client: ignore wl_seat::WlSeat);
delegate_noop!(Client: ignore wl_compositor::WlCompositor);
delegate_noop!(Client: ignore wl_surface::WlSurface);

use wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::{
    zwp_keyboard_shortcuts_inhibit_manager_v1 as inhibit_manager,
    zwp_keyboard_shortcuts_inhibitor_v1 as keyboard_inhibitor,
};
delegate_noop!(Client: ignore inhibit_manager::ZwpKeyboardShortcutsInhibitManagerV1);
impl Dispatch<keyboard_inhibitor::ZwpKeyboardShortcutsInhibitorV1, ()> for Client {
    fn event(
        state: &mut Self,
        _: &keyboard_inhibitor::ZwpKeyboardShortcutsInhibitorV1,
        event: keyboard_inhibitor::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            keyboard_inhibitor::Event::Active => state.inhibitor_states.push(true),
            keyboard_inhibitor::Event::Inactive => state.inhibitor_states.push(false),
            _ => {}
        }
    }
}

type Inspect = Box<dyn FnOnce(&mut Villain) + Send>;
fn inspect(sender: &mpsc::Sender<Inspect>, f: impl FnOnce(&mut Villain) + Send + 'static) {
    let (done, receive) = mpsc::channel();
    sender
        .send(Box::new(move |state| {
            f(state);
            done.send(()).unwrap();
        }))
        .unwrap();
    receive.recv_timeout(Duration::from_secs(5)).unwrap();
}
fn has_global(globals: &GlobalList, interface: &str) -> bool {
    globals
        .contents()
        .with_list(|entries| entries.iter().any(|entry| entry.interface == interface))
}
struct ConfigFileGuard {
    path: PathBuf,
    original: Option<Vec<u8>>,
}
impl ConfigFileGuard {
    /// Preserve the user's configuration file for a test that needs reloads.
    fn new() -> Self {
        let path = knave_config::ConfigDocument::default_path().unwrap();
        let original = match std::fs::read(&path) {
            Ok(contents) => Some(contents),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => panic!("could not preserve {}: {error}", path.display()),
        };
        Self { path, original }
    }

    /// Replace the guarded file with test configuration.
    fn write(&self, contents: &str) {
        std::fs::create_dir_all(self.path.parent().unwrap()).unwrap();
        std::fs::write(&self.path, contents).unwrap();
    }
}
impl Drop for ConfigFileGuard {
    /// Restore the file contents or remove a file created by the test.
    fn drop(&mut self) {
        if let Some(contents) = &self.original {
            let _ = std::fs::write(&self.path, contents);
        } else {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
fn find(state: &Villain, app: &str) -> WindowInfo {
    state
        .window_info()
        .into_iter()
        .find(|window| window.app_id == app)
        .unwrap()
}
fn mapped(state: &Villain, app: &str) -> bool {
    let info = find(state, app);
    state
        .workspace_layout(info.workspace.0 as usize - 1)
        .into_iter()
        .find(|(window, _, _, _, _, _)| {
            state.workspaces[info.workspace.0 as usize - 1]
                .windows
                .iter()
                .any(|entry| entry.id == info.id && entry.window == *window)
        })
        .is_some_and(|(window, _, _, _, _, visible)| {
            visible && state.space.element_location(&window).is_some()
        })
}

fn finish_opening_frames(state: &Villain) {
    let output = smithay::output::Output::new(
        "opening-test".into(),
        smithay::output::PhysicalProperties {
            size: (0, 0).into(),
            subpixel: smithay::output::Subpixel::Unknown,
            make: "test".into(),
            model: "test".into(),
        },
    );
    for window in state.space.elements() {
        Villain::send_window_frame(window, &output, state.start_time.elapsed());
    }
}

#[test]
#[ignore = "requires private Unix sockets and an isolated XDG_RUNTIME_DIR"]
fn wayland_floating_and_fullscreen_requests() {
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
        let create = |id, name: &str| {
            let surface = compositor.create_surface(&qh, ());
            let xdg = wm.get_xdg_surface(&surface, &qh, ());
            let top = xdg.get_toplevel(&qh, id);
            top.set_app_id(name.into());
            (surface, xdg, top)
        };
        let settle = |queue: &mut wayland_client::EventQueue<Client>, client: &mut Client| {
            queue.roundtrip(client).unwrap();
            queue.roundtrip(client).unwrap();
        };
        let (a, _ax, at) = create(1, "primary");
        at.set_maximized();
        a.commit();
        let (b, _bx, bt) = create(2, "other");
        b.commit();
        settle(&mut queue, &mut client);
        inspect(&sender, |state| {
            assert!(find(state, "other").focused);
            assert!(!find(state, "primary").focused);
            // The pointer starts over the first tiled slot. Returning to the
            // workspace must still restore the remembered focus instead of
            // letting that stale hit-test win.
            state.switch_workspace(1);
            state.switch_workspace(0);
            state.refresh_pointer(0);
            assert!(find(state, "other").focused);
        });
        assert_eq!(
            (client.configures[&1].width, client.configures[&1].height),
            (400, 600)
        );
        assert!(client.configures[&1].tiled);
        assert!(
            !client.configures[&1].maximized,
            "startup maximize is rejected"
        );
        inspect(&sender, |state| assert!(!find(state, "primary").maximized));
        // Initial configure acknowledgement alone does not finish opening.
        at.set_maximized();
        settle(&mut queue, &mut client);
        assert!(!client.configures[&1].maximized);
        // Attach real buffers so pointer hit testing follows the normal client path.
        let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).unwrap();
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ()).unwrap();
        let pointer = seat.get_pointer(&qh, ());
        let gestures: zwp_pointer_gestures_v1::ZwpPointerGesturesV1 =
            globals.bind(&qh, 1..=3, ()).unwrap();
        let _swipe = gestures.get_swipe_gesture(&pointer, &qh, ());
        let _pinch = gestures.get_pinch_gesture(&pointer, &qh, ());
        let _hold = gestures.get_hold_gesture(&pointer, &qh, ());
        let path =
            std::env::temp_dir().join(format!("villain-split-buffer-{}", std::process::id()));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        std::fs::remove_file(path).unwrap();
        file.set_len(800 * 600 * 4).unwrap();
        let pool = shm.create_pool(file.as_fd(), 800 * 600 * 4, &qh, ());
        let buffer = pool.create_buffer(0, 400, 600, 400 * 4, wl_shm::Format::Argb8888, &qh, ());
        a.attach(Some(&buffer), 0, 0);
        a.commit();
        b.attach(Some(&buffer), 0, 0);
        b.commit();
        settle(&mut queue, &mut client);
        // Toolkits may send startup maximize after a placeholder buffer.
        at.set_maximized();
        settle(&mut queue, &mut client);
        assert!(!client.configures[&1].maximized);
        a.frame(&qh, ());
        a.commit();
        b.frame(&qh, ());
        b.commit();
        settle(&mut queue, &mut client);
        inspect(&sender, |state| finish_opening_frames(state));
        settle(&mut queue, &mut client);
        std::thread::sleep(Duration::from_secs(1));
        inspect(&sender, |s| {
            s.pointer_location = (100.0, 100.0).into();
            s.refresh_pointer(0);
            s.swipe_begin("test".into(), 4, 1);
            s.swipe_update("test", -100.0, 0.0, 2);
            s.swipe_end("test", false, 3);
        });
        settle(&mut queue, &mut client);
        assert_eq!(client.swipes, ["begin", "update", "end"]);
        client.swipes.clear();
        inspect(&sender, |s| {
            s.pointer.clone().gesture_pinch_begin(
                s,
                &smithay::input::pointer::GesturePinchBeginEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time: 4,
                    fingers: 2,
                },
            );
            s.pointer.clone().gesture_pinch_update(
                s,
                &smithay::input::pointer::GesturePinchUpdateEvent {
                    time: 5,
                    delta: (3.0, 4.0).into(),
                    scale: 1.25,
                    rotation: 8.0,
                },
            );
            s.pointer.clone().gesture_pinch_end(
                s,
                &smithay::input::pointer::GesturePinchEndEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time: 6,
                    cancelled: false,
                },
            );
            s.pointer.clone().gesture_hold_begin(
                s,
                &smithay::input::pointer::GestureHoldBeginEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time: 7,
                    fingers: 3,
                },
            );
            s.pointer.clone().gesture_hold_end(
                s,
                &smithay::input::pointer::GestureHoldEndEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time: 8,
                    cancelled: false,
                },
            );
        });
        settle(&mut queue, &mut client);
        assert_eq!(client.pinches, ["begin", "update", "end"]);
        assert_eq!(client.holds, ["begin", "end"]);
        inspect(&sender, |s| {
            s.swipe_begin("test".into(), 3, 4);
            s.swipe_update("test", -60.0, 0.0, 5);
            assert_eq!(s.gestures.transition.unwrap().offset, -240.0);
            s.swipe_end("test", true, 6);
            assert_eq!(s.active_workspace, 0);
            s.cancel_swipe(7);
        });
        settle(&mut queue, &mut client);
        assert!(
            client.swipes.is_empty(),
            "reserved streams never reach the client"
        );
        inspect(&sender, |s| {
            s.config.gestures[0].action = knave_config::gesture::GestureAction::None;
            s.swipe_begin("test".into(), 3, 8);
            s.swipe_update("test", -60.0, 0.0, 9);
            s.cancel_swipe(10);
            s.swipe_update("test", -60.0, 0.0, 11);
            s.swipe_end("test", false, 12);
            s.config.gestures[0].action = knave_config::gesture::GestureAction::Workspace;
        });
        settle(&mut queue, &mut client);
        assert_eq!(client.swipes, ["begin", "update", "cancel"]);
        inspect(&sender, |s| {
            let focus = s.keyboard.current_focus();
            s.pointer_location = (2000.0, 100.0).into();
            s.refresh_pointer_and_focus(0);
            assert_eq!(
                s.keyboard.current_focus(),
                focus,
                "empty clicks retain focus"
            );
            assert!(find(s, "other").focused);
            s.keyboard
                .clone()
                .set_focus(s, None, SERIAL_COUNTER.next_serial());
            s.refresh_pointer_and_focus(0);
            assert!(
                find(s, "other").focused,
                "empty clicks recover missing focus"
            );
            s.switch_workspace(1);
            s.refresh_pointer_and_focus(0);
            assert!(s.keyboard.current_focus().is_none());
            s.switch_workspace(0);
            s.host_focused = false;
            s.refresh_pointer_and_focus(0);
            assert!(s.keyboard.current_focus().is_none());
            s.host_focused = true;
            s.restore_active_workspace_focus();
            s.pointer_location = (100.0, 100.0).into();
            s.refresh_pointer_and_focus(0);
            assert!(
                find(s, "primary").focused,
                "window clicks still change focus"
            );
            s.focus_window(find(s, "other").id);
            use crate::dispatch::{Dispatch, FocusCycleStep};
            let primary = find(s, "primary").id;
            s.suppressed_keys
                .insert(smithay::input::keyboard::Keycode::new(125));
            s.cycle_focus(FocusCycleStep::Next);
            assert_eq!(s.active_window_info().unwrap().id, primary);
            s.suppressed_keys.clear();
            assert!(find(s, "primary").focused);
            use smithay::backend::input::ButtonState;
            let focus = s.keyboard.current_focus();
            s.dispatch(Dispatch::ResizeMaster(10)).unwrap();
            assert_eq!(s.master_ratio(0), 6000);
            assert_eq!(s.workspace_layout(0)[0].1.size.w, 480);
            assert_eq!(s.keyboard.current_focus(), focus);
            s.switch_workspace(1);
            assert_eq!(s.master_ratio(1), 5000);
            s.switch_workspace(0);
            assert_eq!(s.master_ratio(0), 6000);
            s.dispatch(Dispatch::ResetMaster).unwrap();
            s.pointer_location = (402.0, 100.0).into();
            s.refresh_pointer(1);
            s.swipe_begin("test".into(), 3, 18);
            s.swipe_update("test", -100.0, 0.0, 19);
            assert!(s.gestures.transition.is_some());
            s.pressed_buttons.insert(0x110);
            assert!(s.try_start_split_drag(0x110, ButtonState::Pressed, 1));
            assert!(s.pointer.current_focus().is_none());
            s.maintain_swipe();
            s.swipe_update("test", -100.0, 0.0, 20);
            s.swipe_end("test", false, 21);
            assert!(
                s.gestures.transition.is_none(),
                "new split grab cancels active swipe"
            );
            s.swipe_begin("test".into(), 3, 20);
            s.swipe_update("test", -100.0, 0.0, 21);
            s.swipe_end("test", false, 22);
            assert!(s.gestures.transition.is_none(), "split grab owns input");
            assert_eq!(s.active_workspace, 0);
            s.pointer_location = (642.0, 100.0).into();
            s.refresh_pointer(2);
            assert_eq!(s.master_ratio(0), 8000);
            s.pointer_location = (2000.0, 100.0).into();
            s.refresh_pointer(3);
            assert_eq!(s.master_ratio(0), 9000);
            s.pointer.clone().button(
                s,
                &smithay::input::pointer::ButtonEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time: 4,
                    button: 0x110,
                    state: ButtonState::Released,
                },
            );
            s.pressed_buttons.remove(&0x110);
            assert!(!s.split_drag_active && !s.pointer.is_grabbed());
            assert_eq!(s.keyboard.current_focus(), focus);
            // Output changes cancel, preserving the already chosen proportion.
            s.pointer_location = (722.0, 100.0).into();
            s.refresh_pointer(5);
            s.pressed_buttons.insert(0x110);
            assert!(s.try_start_split_drag(0x110, ButtonState::Pressed, 5));
            s.output_size = (1000, 600).into();
            s.relayout_active_workspace();
            assert!(!s.split_drag_active);
            assert_eq!(s.master_ratio(0), 9000);
            s.release_pointer_buttons();
            s.output_size = (800, 600).into();
            s.relayout_active_workspace();
            s.dispatch(Dispatch::ResetMaster).unwrap();
            // A workspace transition also releases the grab and held button.
            s.pointer_location = (402.0, 100.0).into();
            s.refresh_pointer(6);
            s.pressed_buttons.insert(0x110);
            assert!(s.try_start_split_drag(0x110, ButtonState::Pressed, 6));
            s.switch_workspace(1);
            assert!(!s.split_drag_active && s.pressed_buttons.is_empty());
            s.switch_workspace(0);
            let window = s.workspaces[0].windows[0].window.clone();
            s.set_window_maximized(&window, true);
            assert!(s.split_context().is_none());
            s.dispatch(Dispatch::ResizeMaster(10)).unwrap();
            assert_eq!(s.master_ratio(0), 5000);
            s.set_window_maximized(&window, false);
            // Defaults reload without overwriting a workspace's manual choice.
            let path = knave_config::ConfigDocument::default_path().unwrap();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "[compositor]\nmaster_percent = 60\n").unwrap();
            s.dispatch(Dispatch::ReloadConfig).unwrap();
            assert_eq!(s.master_ratio(0), 6000);
            s.dispatch(Dispatch::ResizeMaster(5)).unwrap();
            std::fs::write(&path, "[compositor]\nmaster_percent = 70\n").unwrap();
            s.dispatch(Dispatch::ReloadConfig).unwrap();
            assert_eq!(s.master_ratio(0), 6500);
            s.dispatch(Dispatch::ResetMaster).unwrap();
            assert_eq!(s.master_ratio(0), 7000);
            std::fs::remove_file(path).unwrap();
            s.dispatch(Dispatch::ReloadConfig).unwrap();
            assert_eq!(s.master_ratio(0), 5000);
            // Repeated adjustment must not accumulate beyond the bounds.
            for _ in 0..250 {
                s.dispatch(Dispatch::ResizeMaster(5)).unwrap();
            }
            assert_eq!(s.master_ratio(0), 9000);
            s.dispatch(Dispatch::ResetMaster).unwrap();
        });
        settle(&mut queue, &mut client);
        assert_eq!(client.configures[&1].width, 400);
        assert_eq!(
            client.pointer_buttons, 0,
            "divider presses must not reach clients"
        );
        inspect(&sender, |s| {
            use knave_config::appearance::{Corners, Edges, WindowAppearance};
            s.config.appearance.gaps.outer = Edges {
                top: 10,
                right: 20,
                bottom: 30,
                left: 40,
            };
            s.config.appearance.gaps.inner = Edges {
                top: 2,
                right: 3,
                bottom: 4,
                left: 5,
            };
            s.config.appearance.border.width = Edges {
                top: 4,
                right: 4,
                bottom: 4,
                left: 4,
            };
            s.config.appearance.border.radius = Corners {
                top_left: 12,
                top_right: 20,
                bottom_right: 8,
                bottom_left: 0,
            };
            s.relayout_active_workspace();
            let layout = s.workspace_layout(0);
            let first = layout[0].0.clone();
            assert_eq!(
                layout[0].1,
                Rectangle::new((49, 16).into(), (354, 546).into())
            );
            assert_eq!(
                layout[1].1,
                Rectangle::new((419, 16).into(), (354, 546).into())
            );
            s.pointer_location = (75.0, 14.0).into();
            s.focus_window_at_pointer();
            assert_eq!(
                s.keyboard.current_focus(),
                KeyboardFocus::for_window(&first)
            );
            s.pointer_location = (49.5, 16.5).into();
            assert!(
                s.input_window_at_pointer().is_none(),
                "clipped content corners must reject input"
            );
            // Gaps must still admit the compositor-owned split grab.
            s.pointer_location = (410.0, 100.0).into();
            assert!(s.split_under_pointer().is_some());
            s.set_window_maximized(&first, true);
            assert_eq!(
                s.workspace_layout(0)
                    .into_iter()
                    .find(|entry| entry.0 == first)
                    .unwrap()
                    .1,
                Rectangle::from_size((800, 600).into())
            );
            s.config.appearance.views.maximized.gaps = true;
            s.config.appearance.views.maximized.borders = true;
            s.relayout_active_workspace();
            assert_eq!(
                s.workspace_layout(0)
                    .into_iter()
                    .find(|entry| entry.0 == first)
                    .unwrap()
                    .1,
                Rectangle::new((44, 14).into(), (732, 552).into())
            );
            s.set_window_fullscreen(&first, true);
            assert_eq!(
                s.workspace_layout(0)
                    .into_iter()
                    .find(|entry| entry.0 == first)
                    .unwrap()
                    .1,
                Rectangle::from_size((800, 600).into())
            );
            s.set_window_fullscreen(&first, false);
            s.set_window_maximized(&first, false);
            s.config.appearance = WindowAppearance::default();
            s.relayout_active_workspace();
        });
        settle(&mut queue, &mut client);
        let (dialog, _dx, dt) = create(3, "dialog");
        dt.set_parent(Some(&at));
        dialog.commit();
        let (fixed, _fx, ft) = create(4, "fixed");
        ft.set_min_size(240, 180);
        ft.set_max_size(240, 180);
        fixed.commit();
        settle(&mut queue, &mut client);
        inspect(&sender, |state| {
            assert!(find(state, "dialog").floating);
            assert!(find(state, "fixed").floating);
            assert!(mapped(state, "primary") && mapped(state, "other") && mapped(state, "dialog"));
        });
        assert_eq!(
            (client.configures[&4].width, client.configures[&4].height),
            (240, 180)
        );
        assert!(!client.configures[&3].tiled);
        let dialog_size = (client.configures[&3].width, client.configures[&3].height);
        // Maximization retains the layout and dialogs, and is independent of fullscreen.
        at.set_maximized();
        settle(&mut queue, &mut client);
        assert!(client.configures[&1].maximized);
        assert!(!client.configures[&1].fullscreen && !client.configures[&1].tiled);
        assert_eq!(
            (client.configures[&1].width, client.configures[&1].height),
            (800, 600)
        );
        inspect(&sender, |state| {
            assert!(find(state, "primary").maximized);
            assert!(mapped(state, "primary") && mapped(state, "dialog"));
            assert!(!mapped(state, "other"));
            state.focus_window(find(state, "dialog").id);
            assert!(
                find(state, "primary").maximized,
                "child focus retains parent maximization"
            );
        });
        // Initial parent hints arrive after get_toplevel: a new child must not
        // restore its maximized parent, but a new unrelated application must.
        let (child, child_xdg, child_top) = create(6, "new-child");
        child_top.set_parent(Some(&at));
        child.commit();
        settle(&mut queue, &mut client);
        inspect(&sender, |state| {
            assert!(find(state, "primary").maximized);
            assert!(mapped(state, "new-child"));
            assert!(find(state, "new-child").focused);
        });
        child_top.destroy();
        child_xdg.destroy();
        child.destroy();
        settle(&mut queue, &mut client);
        inspect(&sender, |state| {
            state
                .suppressed_keys
                .insert(smithay::input::keyboard::Keycode::new(125));
        });
        let (launched, launched_xdg, launched_top) = create(7, "new-app");
        launched.commit();
        settle(&mut queue, &mut client);
        inspect(&sender, |state| {
            assert!(!find(state, "primary").maximized);
            assert!(mapped(state, "new-app"));
            assert!(find(state, "new-app").focused);
            state.suppressed_keys.clear();
            assert!(find(state, "new-app").focused);
            assert!(mapped(state, "primary") && mapped(state, "other"));
        });
        launched_top.destroy();
        launched_xdg.destroy();
        launched.destroy();
        settle(&mut queue, &mut client);
        at.set_maximized();
        settle(&mut queue, &mut client);
        b.commit();
        settle(&mut queue, &mut client);
        inspect(&sender, |state| {
            assert!(find(state, "primary").maximized);
            assert!(!mapped(state, "other"));
        });
        // A late first commit on a workspace we have left must not change it.
        let (late_map, late_map_xdg, late_map_top) = create(8, "late-map");
        settle(&mut queue, &mut client);
        inspect(&sender, |state| state.switch_workspace(4));
        late_map.commit();
        settle(&mut queue, &mut client);
        inspect(&sender, |state| {
            assert_eq!(state.active_workspace, 4);
            assert!(find(state, "primary").maximized);
            assert!(!mapped(state, "late-map"));
        });
        late_map_top.destroy();
        late_map_xdg.destroy();
        late_map.destroy();
        settle(&mut queue, &mut client);
        inspect(&sender, |state| state.switch_workspace(0));
        client.configures.remove(&1);
        at.set_maximized();
        settle(&mut queue, &mut client);
        assert!(
            client.configures[&1].maximized,
            "repeated requests receive configure"
        );
        at.set_fullscreen(None);
        settle(&mut queue, &mut client);
        assert!(client.configures[&1].fullscreen && client.configures[&1].maximized);
        let (full_hidden, full_hidden_xdg, full_hidden_top) = create(9, "fullscreen-hidden");
        full_hidden.commit();
        settle(&mut queue, &mut client);
        inspect(&sender, |state| {
            assert!(find(state, "primary").fullscreen && find(state, "primary").maximized);
            assert!(!mapped(state, "fullscreen-hidden"));
        });
        full_hidden_top.destroy();
        full_hidden_xdg.destroy();
        full_hidden.destroy();
        settle(&mut queue, &mut client);
        at.unset_fullscreen();
        settle(&mut queue, &mut client);
        assert!(!client.configures[&1].fullscreen && client.configures[&1].maximized);
        at.unset_maximized();
        settle(&mut queue, &mut client);
        assert!(!client.configures[&1].maximized && client.configures[&1].tiled);
        assert_eq!(client.configures[&1].width, 400);
        // A floating dialog retains its exact pre-maximize geometry.
        dialog.attach(Some(&buffer), 0, 0);
        dialog.frame(&qh, ());
        dialog.commit();
        settle(&mut queue, &mut client);
        inspect(&sender, |state| finish_opening_frames(state));
        settle(&mut queue, &mut client);
        std::thread::sleep(Duration::from_secs(1));
        dt.set_maximized();
        settle(&mut queue, &mut client);
        assert!(client.configures[&3].maximized);
        dt.unset_maximized();
        settle(&mut queue, &mut client);
        assert_eq!(
            (client.configures[&3].width, client.configures[&3].height),
            dialog_size
        );
        inspect(&sender, |state| {
            use crate::dispatch::Dispatch;
            let primary = find(state, "primary").id;
            state.focus_window(primary);
            state.dispatch(Dispatch::MaximizeFocused).unwrap();
            assert!(state.minimize_focused_window());
            assert!(find(state, "primary").maximized && mapped(state, "other"));
            assert!(state.restore_last_minimized_window());
            assert!(find(state, "primary").maximized && mapped(state, "primary"));
            assert!(!mapped(state, "other"));
            state.focus_window(find(state, "other").id);
            assert!(!find(state, "primary").maximized);
            assert!(mapped(state, "primary") && mapped(state, "other"));
            state.dispatch(Dispatch::ToggleMaximizeFocused).unwrap();
            assert!(find(state, "other").maximized);
            state.dispatch(Dispatch::ToggleMaximizeFocused).unwrap();
            assert!(!find(state, "other").maximized);
            state.dispatch(Dispatch::UnmaximizeFocused).unwrap();
            state.switch_workspace(4);
            assert!(state.dispatch(Dispatch::MaximizeFocused).is_err());
        });
        // Background requests configure only their workspace and do not steal focus.
        bt.set_maximized();
        settle(&mut queue, &mut client);
        assert!(client.configures[&2].maximized);
        inspect(&sender, |state| {
            assert_eq!(state.active_workspace, 4);
            assert!(!mapped(state, "other"));
            state.switch_workspace(0);
            assert!(mapped(state, "other") && !mapped(state, "primary"));
            assert!(state.maximize_focused_window(Some(false)));
        });
        settle(&mut queue, &mut client);

        at.set_fullscreen(None);
        settle(&mut queue, &mut client);
        assert!(client.configures[&1].fullscreen && !client.configures[&1].tiled);
        client.configures.remove(&1);
        at.set_fullscreen(None);
        settle(&mut queue, &mut client);
        assert!(
            client.configures[&1].fullscreen,
            "repeated request needs a configure response"
        );
        assert_eq!(
            (client.configures[&1].width, client.configures[&1].height),
            (800, 600)
        );
        inspect(&sender, |state| {
            assert!(mapped(state, "primary") && mapped(state, "dialog"));
            assert!(!mapped(state, "other") && !mapped(state, "fixed"));
            assert_eq!(state.workspace_info()[0].visible_window_count, 2);
        });
        // A floating window restores its old size after fullscreen.
        dt.set_fullscreen(None);
        settle(&mut queue, &mut client);
        assert!(client.configures[&3].fullscreen);
        assert!(!client.configures[&1].fullscreen);
        inspect(&sender, |state| {
            let id = find(state, "primary").id;
            let parent = state.workspaces[0]
                .windows
                .iter()
                .find(|entry| entry.id == id)
                .unwrap()
                .window
                .clone();
            state.apply_window_action(&parent, WindowAction::Minimize);
            assert!(mapped(state, "other"));
            assert!(!mapped(state, "dialog"));
            assert_eq!(state.focus_window(find(state, "dialog").id), Some(false));
            assert!(state.restore_last_minimized_window());
            assert!(mapped(state, "dialog"));
        });
        dt.unset_fullscreen();
        settle(&mut queue, &mut client);
        assert_eq!(
            (client.configures[&3].width, client.configures[&3].height),
            dialog_size
        );
        assert!(!client.configures[&3].fullscreen);
        assert_eq!(
            (client.configures[&1].width, client.configures[&1].height),
            (400, 600)
        );
        inspect(&sender, |state| {
            assert!(mapped(state, "other") && mapped(state, "fixed"))
        });
        // Relaxing committed fixed-size hints returns a standalone window to tiling.
        ft.set_min_size(0, 0);
        ft.set_max_size(0, 0);
        fixed.commit();
        settle(&mut queue, &mut client);
        inspect(&sender, |state| assert!(!find(state, "fixed").floating));
        // Fullscreen on an inactive workspace must configure without switching it.
        inspect(&sender, |state| state.switch_workspace(4));
        bt.set_fullscreen(None);
        settle(&mut queue, &mut client);
        assert!(client.configures[&2].fullscreen);
        inspect(&sender, |state| {
            assert_eq!(state.active_workspace, 4);
            assert!(find(state, "other").fullscreen);
            assert!(!mapped(state, "other"));
            state.switch_workspace(0);
            assert!(mapped(state, "other") && !mapped(state, "primary"));
            let id = find(state, "other").id;
            assert_eq!(state.focus_window(id), Some(true));
            assert!(state.minimize_focused_window());
            assert!(mapped(state, "primary"));
            assert!(state.restore_last_minimized_window());
            assert!(mapped(state, "other") && !mapped(state, "primary"));
            // An explicit user focus request makes the selected window reachable.
            let id = find(state, "primary").id;
            state.focus_window(id);
            assert!(mapped(state, "primary"));
            assert!(!find(state, "other").fullscreen);
            state.switch_workspace(4);
            state.switch_workspace(0);
            assert!(find(state, "primary").focused);
        });
        // Set a parent after creation while viewing a different workspace.
        inspect(&sender, |state| state.switch_workspace(4));
        let (late, _lx, lt) = create(5, "late-dialog");
        lt.set_parent(Some(&at));
        late.commit();
        settle(&mut queue, &mut client);
        inspect(&sender, |state| {
            assert_eq!(find(state, "late-dialog").workspace.0, 1);
            assert!(find(state, "late-dialog").floating);
            assert!(!mapped(state, "late-dialog"));
        });
        at.set_maximized();
        at.set_fullscreen(None);
        settle(&mut queue, &mut client);
        at.destroy();
        _ax.destroy();
        a.destroy();
        settle(&mut queue, &mut client);
        inspect(&sender, |state| {
            state.switch_workspace(0);
            assert!(mapped(state, "other"));
            assert!(!state.workspace_has_fullscreen(0));
            assert!(state.workspaces[0].maximized.is_none());
            assert!(find(state, "other").focused);
        });
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    while !thread.is_finished() {
        assert!(Instant::now() < deadline, "Wayland request test timed out");
        event_loop
            .dispatch(Duration::from_millis(2), &mut state)
            .unwrap();
        while let Ok(inspect) = requests.try_recv() {
            inspect(&mut state);
        }
        state.space.refresh();
        state.display_handle.flush_clients().unwrap();
    }
    thread.join().unwrap();
}

#[test]
#[ignore = "requires private Unix sockets and an isolated XDG_RUNTIME_DIR"]
fn focus_cycle_wraps_skips_minimized_and_leaves_maximize() {
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
        let create = |id, name: &str| {
            let surface = compositor.create_surface(&qh, ());
            let xdg = wm.get_xdg_surface(&surface, &qh, ());
            let top = xdg.get_toplevel(&qh, id);
            top.set_app_id(name.into());
            (surface, xdg, top)
        };
        let settle = |queue: &mut wayland_client::EventQueue<Client>, client: &mut Client| {
            queue.roundtrip(client).unwrap();
            queue.roundtrip(client).unwrap();
        };
        let (a, _ax, _at) = create(1, "primary");
        a.commit();
        let (b, _bx, _bt) = create(2, "other");
        b.commit();
        settle(&mut queue, &mut client);
        // Newest mapped window wins initial focus.
        inspect(&sender, |state| {
            use crate::dispatch::{Dispatch, FocusCycleStep};
            assert!(find(state, "other").focused);

            // Layout order: master first, then the stack. Wrapping is clean.
            state
                .dispatch(Dispatch::FocusCycle(FocusCycleStep::Previous))
                .unwrap();
            assert!(find(state, "primary").focused);
            state
                .dispatch(Dispatch::FocusCycle(FocusCycleStep::Next))
                .unwrap();
            assert!(find(state, "other").focused);

            // Minimized windows are not targets: the cycle keeps skipping them.
            let id = find(state, "primary").id;
            state.focus_window(id);
            assert!(state.minimize_focused_window());
            state
                .dispatch(Dispatch::FocusCycle(FocusCycleStep::Next))
                .unwrap();
            assert!(find(state, "other").focused);
            state.dispatch(Dispatch::RestoreLastMinimized).unwrap();
            state.focus_window(find(state, "primary").id);

            // Focus switching must leave a maximize overlay: the target is
            // revealed by un-maximizing the blocker.
            state.set_window_maximized(&state.workspaces[0].windows[0].window.clone(), true);
            assert!(!mapped(state, "other"));
            state
                .dispatch(Dispatch::FocusCycle(FocusCycleStep::Next))
                .unwrap();
            assert!(find(state, "other").focused);
            assert!(!find(state, "primary").maximized);
            assert!(mapped(state, "other") && mapped(state, "primary"));
        });
        settle(&mut queue, &mut client);
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    while !thread.is_finished() {
        assert!(Instant::now() < deadline, "Wayland request test timed out");
        event_loop
            .dispatch(Duration::from_millis(2), &mut state)
            .unwrap();
        while let Ok(inspect) = requests.try_recv() {
            inspect(&mut state);
        }
        state.space.refresh();
        state.display_handle.flush_clients().unwrap();
    }
    thread.join().unwrap();
}

#[test]
#[ignore = "requires private Unix sockets and an isolated XDG_RUNTIME_DIR"]
fn directional_focus_and_cycle_order_follow_configuration() {
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
        let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).unwrap();
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ()).unwrap();
        let _keyboard = seat.get_keyboard(&qh, ());
        let mut client = Client::default();
        let create = |id, name: &str| {
            let surface = compositor.create_surface(&qh, ());
            let xdg = wm.get_xdg_surface(&surface, &qh, ());
            let top = xdg.get_toplevel(&qh, id);
            top.set_app_id(name.into());
            (surface, xdg, top)
        };
        let settle = |queue: &mut wayland_client::EventQueue<Client>, client: &mut Client| {
            queue.roundtrip(client).unwrap();
            queue.roundtrip(client).unwrap();
        };
        // Attach a 400x600 buffer everywhere so pointer hit testing follows
        // the normal client path.
        let buffer = {
            let path =
                std::env::temp_dir().join(format!("villain-focus-tests-{}", std::process::id()));
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            std::fs::remove_file(path).unwrap();
            file.set_len(400 * 600 * 4).unwrap();
            let pool = shm.create_pool(file.as_fd(), 400 * 600 * 4, &qh, ());
            pool.create_buffer(0, 400, 600, 400 * 4, wl_shm::Format::Argb8888, &qh, ())
        };
        use crate::dispatch::{Dispatch, FocusCycleStep, FocusDirection};
        let (a, _ax, _at) = create(1, "primary");
        a.attach(Some(&buffer), 0, 0);
        a.commit();
        let (b, _bx, _bt) = create(2, "other");
        b.attach(Some(&buffer), 0, 0);
        b.commit();
        let (c, _cx, _ct) = create(3, "tertiary");
        c.attach(Some(&buffer), 0, 0);
        c.commit();
        settle(&mut queue, &mut client);
        inspect(&sender, |state| {
            // Master left; two stack slices top and bottom. Right from the
            // master hits an equal-distance tie between the slices, which
            // layout order resolves to the first candidate.
            state.focus_window(find(state, "primary").id);
            state
                .dispatch(Dispatch::FocusDirection(FocusDirection::Right))
                .unwrap();
            assert!(find(state, "other").focused);
            state
                .dispatch(Dispatch::FocusDirection(FocusDirection::Down))
                .unwrap();
            assert!(find(state, "tertiary").focused);
            state
                .dispatch(Dispatch::FocusDirection(FocusDirection::Up))
                .unwrap();
            assert!(find(state, "other").focused);
            state
                .dispatch(Dispatch::FocusDirection(FocusDirection::Up))
                .unwrap();
            assert!(
                find(state, "other").focused,
                "nothing lies above the top slice: the edge is a no-op"
            );
            state
                .dispatch(Dispatch::FocusDirection(FocusDirection::Left))
                .unwrap();
            assert!(find(state, "primary").focused);
            state
                .dispatch(Dispatch::FocusDirection(FocusDirection::Left))
                .unwrap();
            assert!(
                find(state, "primary").focused,
                "nothing lies left of the master: the edge is a no-op"
            );

            // Layout order walks primary, other, tertiary.
            state.focus_window(find(state, "tertiary").id);
            state.focus_window(find(state, "primary").id);
            state
                .dispatch(Dispatch::FocusCycle(FocusCycleStep::Next))
                .unwrap();
            assert!(find(state, "other").focused);
            state.focus_window(find(state, "primary").id);

            // MRU order walks most recently focused first, so Next from the
            // master targets tertiary, the second-newest focus. The recency
            // order is only rewritten by ordinary focus changes, so repeated
            // presses keep walking the last order.
            state.config.focus_cycle_order = knave_config::FocusCycleOrder::Mru;
            state
                .dispatch(Dispatch::FocusCycle(FocusCycleStep::Next))
                .unwrap();
            assert!(find(state, "tertiary").focused);
            state
                .dispatch(Dispatch::FocusCycle(FocusCycleStep::Next))
                .unwrap();
            assert!(find(state, "other").focused);
            state
                .dispatch(Dispatch::FocusCycle(FocusCycleStep::Next))
                .unwrap();
            assert!(find(state, "primary").focused, "mru cycles wrap");
            state
                .dispatch(Dispatch::FocusCycle(FocusCycleStep::Previous))
                .unwrap();
            assert!(find(state, "other").focused);
            state
                .dispatch(Dispatch::FocusCycle(FocusCycleStep::Previous))
                .unwrap();
            assert!(find(state, "tertiary").focused);
            state.config.focus_cycle_order = knave_config::FocusCycleOrder::Layout;

            // Physical Super stays held across Tab taps and direction changes.
            state.focus_window(find(state, "primary").id);
            use crate::keybinds::handle_keyboard_key;
            use smithay::{
                backend::input::KeyState::{Pressed, Released},
                input::keyboard::Keycode,
            };
            handle_keyboard_key(state, Keycode::new(133), Pressed, 1); // evdev left Super + 8
            for target in ["other", "tertiary", "primary", "other"] {
                handle_keyboard_key(state, Keycode::new(23), Pressed, 2); // Tab
                assert!(
                    find(state, target).focused,
                    "focus changes before either key is released"
                );
                handle_keyboard_key(state, Keycode::new(23), Released, 3);
            }
            handle_keyboard_key(state, Keycode::new(50), Pressed, 4); // Shift
            handle_keyboard_key(state, Keycode::new(23), Pressed, 5);
            assert!(
                find(state, "primary").focused,
                "Shift reverses without releasing Super"
            );
            handle_keyboard_key(state, Keycode::new(23), Released, 6);
            handle_keyboard_key(state, Keycode::new(50), Released, 7);
            handle_keyboard_key(state, Keycode::new(46), Pressed, 8); // L
            assert!(find(state, "other").focused);
            handle_keyboard_key(state, Keycode::new(46), Released, 9);
            handle_keyboard_key(state, Keycode::new(133), Released, 10);
            assert!(state.suppressed_keys.is_empty());
            assert!(!state.keyboard.modifier_state().logo);
        });
        settle(&mut queue, &mut client);
        assert!(
            client.keyboard_modifiers.iter().any(|mods| *mods != 0),
            "held modifiers reach client"
        );
        assert_eq!(
            client.keyboard_modifiers.last(),
            Some(&0),
            "intercepted Super release clears client modifiers"
        );
        assert!(
            client.keyboard_keys.iter().all(|key| *key == 42),
            "shortcut keys never reach client (Shift may be forwarded)"
        );
        assert!(
            client
                .keyboard_enters
                .iter()
                .all(|(_, keys)| keys.is_empty() || keys.as_slice() == 42u32.to_ne_bytes()),
            "enter excludes intercepted shortcut keys"
        );
        inspect(&sender, |state| {
            assert!(find(state, "other").focused);
            let id = find(state, "primary").id;
            state.focus_window(id);
            // Hover-to-focus: window focus follows pointer motion.
            state.config.input.focus_follows_mouse = true;
            state.pointer_location = (600.0, 100.0).into();
            state.refresh_pointer_motion(0);
            assert!(
                find(state, "other").focused,
                "motion over a window focuses it"
            );
            // Click-to-focus: motion refreshes visuals only.
            state.config.input.focus_follows_mouse = false;
            let id = find(state, "primary").id;
            state.focus_window(id);
            state.refresh_pointer_motion(1);
            assert!(
                find(state, "primary").focused,
                "motion keeps focus with focus following off"
            );
            state.config.input.focus_follows_mouse = true;
        });
        settle(&mut queue, &mut client);
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    while !thread.is_finished() {
        assert!(Instant::now() < deadline, "Wayland request test timed out");
        event_loop
            .dispatch(Duration::from_millis(2), &mut state)
            .unwrap();
        while let Ok(inspect) = requests.try_recv() {
            inspect(&mut state);
        }
        state.space.refresh();
        state.display_handle.flush_clients().unwrap();
    }
    thread.join().unwrap();
}

#[test]
#[ignore = "requires private Unix sockets and an isolated XDG_RUNTIME_DIR"]
fn keyboard_shortcuts_inhibitor_forwards_binding_keys() {
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
        let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).unwrap();
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ()).unwrap();
        let _keyboard = seat.get_keyboard(&qh, ());
        let inhibit: inhibit_manager::ZwpKeyboardShortcutsInhibitManagerV1 =
            globals.bind(&qh, 1..=1, ()).unwrap();
        let mut client = Client::default();
        let create = |id, name: &str| {
            let surface = compositor.create_surface(&qh, ());
            let xdg = wm.get_xdg_surface(&surface, &qh, ());
            let top = xdg.get_toplevel(&qh, id);
            top.set_app_id(name.into());
            (surface, xdg, top)
        };
        let settle = |queue: &mut wayland_client::EventQueue<Client>, client: &mut Client| {
            queue.roundtrip(client).unwrap();
            queue.roundtrip(client).unwrap();
        };
        // Real buffers so keyboard focus follows the mapped-window path.
        let buffer = {
            let path =
                std::env::temp_dir().join(format!("villain-inhibit-tests-{}", std::process::id()));
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            std::fs::remove_file(path).unwrap();
            file.set_len(400 * 600 * 4).unwrap();
            let pool = shm.create_pool(file.as_fd(), 400 * 600 * 4, &qh, ());
            pool.create_buffer(0, 400, 600, 400 * 4, wl_shm::Format::Argb8888, &qh, ())
        };
        let (primary, _px, _pt) = create(1, "primary");
        primary.attach(Some(&buffer), 0, 0);
        primary.commit();
        let (grab, _gx, _gt) = create(2, "grabby");
        grab.attach(Some(&buffer), 0, 0);
        grab.commit();
        settle(&mut queue, &mut client);
        inspect(&sender, |state| {
            state.focus_window(find(state, "primary").id);
        });
        settle(&mut queue, &mut client);
        // Inhibition is tied to keyboard focus: the surface is unfocused, so
        // the fresh inhibitor must stay inactive until the surface is entered.
        let inhibitor = inhibit.inhibit_shortcuts(&grab, &seat, &qh, ());
        settle(&mut queue, &mut client);
        assert!(
            client.inhibitor_states.is_empty(),
            "unfocused inhibitors stay inactive"
        );
        inspect(&sender, |state| {
            state.focus_window(find(state, "grabby").id);
        });
        settle(&mut queue, &mut client);
        assert_eq!(
            client.inhibitor_states,
            [true],
            "focus activates the inhibitor"
        );
        use crate::keybinds::{handle_keyboard_key, shortcuts_inhibited};
        use smithay::{
            backend::input::KeyState::{Pressed, Released},
            input::keyboard::Keycode,
        };
        inspect(&sender, |state| {
            assert!(shortcuts_inhibited(state));
            // Super+Q would normally close the focused window.
            handle_keyboard_key(state, Keycode::new(133), Pressed, 1); // Super + 8
            handle_keyboard_key(state, Keycode::new(24), Pressed, 2); // Q + 8
            handle_keyboard_key(state, Keycode::new(24), Released, 3);
            handle_keyboard_key(state, Keycode::new(133), Released, 4);
            assert!(
                state.window_info().iter().any(|w| w.app_id == "grabby"),
                "the chord must not close an inhibited client"
            );
            assert!(!state.overview_visible, "Super did not toggle the overview");
            assert!(state.suppressed_keys.is_empty());
        });
        settle(&mut queue, &mut client);
        assert!(
            client.keyboard_keys.contains(&16) && client.keyboard_keys.contains(&125),
            "inhibited chord keys reach the client as raw evdev codes"
        );
        assert_eq!(client.closes, 0);
        client.keyboard_keys.clear();
        inspect(&sender, |state| {
            let _ = state.focus_window(find(state, "primary").id);
        });
        settle(&mut queue, &mut client);
        // The protocol makes no client notification for focus loss: the
        // inhibitor stays active and merely becomes irrelevant while its
        // surface is unfocused, so the compositor gates forwarding on the
        // focused surface instead.
        assert_eq!(
            client.inhibitor_states,
            [true],
            "losing focus must not emit inactive"
        );
        inspect(&sender, |state| {
            assert!(!shortcuts_inhibited(state), "focus gates forwarding");
            // The inhibitor is active, but the focused surface is not
            // inhibited: compositor bindings apply normally, so Super+Q
            // closes "primary" while the client stays mapped.
            handle_keyboard_key(state, Keycode::new(133), Pressed, 5);
            handle_keyboard_key(state, Keycode::new(24), Pressed, 6);
            handle_keyboard_key(state, Keycode::new(24), Released, 7);
            handle_keyboard_key(state, Keycode::new(133), Released, 8);
            assert!(
                state.window_info().iter().any(|w| w.app_id == "primary"),
                "the close request stays until the client exits"
            );
        });
        settle(&mut queue, &mut client);
        assert!(
            !client.keyboard_keys.contains(&16),
            "unfocused chord is intercepted"
        );
        assert_eq!(client.closes, 1, "the compositor requests a close");
        inspect(&sender, |state| {
            // A bare Super tap alone must still toggle the overview on the
            // uninhibited surface.
            handle_keyboard_key(state, Keycode::new(133), Pressed, 9);
            handle_keyboard_key(state, Keycode::new(133), Released, 10);
            assert!(state.overview_visible, "Super toggled the overview");
            use crate::dispatch::Dispatch;
            let _ = state.dispatch(Dispatch::ToggleOverview);
            let _ = state.focus_window(find(state, "grabby").id);
        });
        settle(&mut queue, &mut client);
        assert_eq!(
            client.inhibitor_states,
            [true],
            "refocusing neither emits inactive nor duplicates active"
        );
        client.keyboard_keys.clear();
        inspect(&sender, |state| {
            assert!(shortcuts_inhibited(state));
            handle_keyboard_key(state, Keycode::new(133), Pressed, 11);
            handle_keyboard_key(state, Keycode::new(24), Pressed, 12);
            handle_keyboard_key(state, Keycode::new(24), Released, 13);
            handle_keyboard_key(state, Keycode::new(133), Released, 14);
            assert!(!state.overview_visible, "Super did not toggle the overview");
        });
        settle(&mut queue, &mut client);
        assert!(
            client.keyboard_keys.contains(&16),
            "activation survives focus changes: keys reach the client"
        );
        assert_eq!(client.closes, 1);
        inhibitor.destroy();
        settle(&mut queue, &mut client);
        client.keyboard_keys.clear();
        inspect(&sender, |state| {
            assert!(!shortcuts_inhibited(state));
            handle_keyboard_key(state, Keycode::new(133), Pressed, 15);
            handle_keyboard_key(state, Keycode::new(24), Pressed, 16);
            handle_keyboard_key(state, Keycode::new(24), Released, 17);
            handle_keyboard_key(state, Keycode::new(133), Released, 18);
        });
        settle(&mut queue, &mut client);
        assert!(
            !client.keyboard_keys.contains(&16),
            "without the inhibitor the chord is intercepted again"
        );
        assert_eq!(client.closes, 2, "the compositor requests a close again");
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    while !thread.is_finished() {
        assert!(Instant::now() < deadline, "Wayland request test timed out");
        event_loop
            .dispatch(Duration::from_millis(2), &mut state)
            .unwrap();
        while let Ok(inspect) = requests.try_recv() {
            inspect(&mut state);
        }
        state.space.refresh();
        state.display_handle.flush_clients().unwrap();
    }
    thread.join().unwrap();
}

#[test]
#[ignore = "requires a private XDG_RUNTIME_DIR"]
/// Verify a connected registry sees the virtual-keyboard global disappear and return on reload.
fn virtual_keyboard_global_tracks_policy_reloads_for_existing_registry() {
    const VIRTUAL_KEYBOARD_MANAGER: &str = "zwp_virtual_keyboard_manager_v1";

    let config_guard = ConfigFileGuard::new();
    config_guard.write("[compositor.input]\nallow_virtual_keyboard = true\n");
    let path = config_guard.path.clone();
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
        assert!(has_global(&globals, VIRTUAL_KEYBOARD_MANAGER));

        let disable_path = path.clone();
        inspect(&sender, move |state| {
            std::fs::write(
                &disable_path,
                "[compositor.input]\nallow_virtual_keyboard = false\n",
            )
            .unwrap();
            state
                .dispatch(crate::dispatch::Dispatch::ReloadConfig)
                .unwrap();
            assert!(
                !state
                    .virtual_keyboard_allowed
                    .load(std::sync::atomic::Ordering::Relaxed)
            );
        });
        queue.roundtrip(&mut Client::default()).unwrap();
        assert!(!has_global(&globals, VIRTUAL_KEYBOARD_MANAGER));

        let enable_path = path.clone();
        inspect(&sender, move |state| {
            std::fs::write(
                &enable_path,
                "[compositor.input]\nallow_virtual_keyboard = true\n",
            )
            .unwrap();
            state
                .dispatch(crate::dispatch::Dispatch::ReloadConfig)
                .unwrap();
            assert!(
                state
                    .virtual_keyboard_allowed
                    .load(std::sync::atomic::Ordering::Relaxed)
            );
        });
        queue.roundtrip(&mut Client::default()).unwrap();
        assert!(has_global(&globals, VIRTUAL_KEYBOARD_MANAGER));
    });

    let deadline = Instant::now() + Duration::from_secs(10);
    while !thread.is_finished() {
        assert!(
            Instant::now() < deadline,
            "virtual-keyboard policy test timed out"
        );
        event_loop
            .dispatch(Duration::from_millis(2), &mut state)
            .unwrap();
        while let Ok(inspect) = requests.try_recv() {
            inspect(&mut state);
        }
        state.display_handle.flush_clients().unwrap();
    }
    thread.join().unwrap();
}

#[path = "tile_tests.rs"]
mod tile_tests;

#[path = "workspace_navigation_tests.rs"]
mod workspace_navigation_tests;

#[path = "drag_tests.rs"]
mod drag_tests;

#[path = "stacking_tests.rs"]
mod stacking_tests;

use wayland_protocols::xdg::decoration::zv1::client::{
    zxdg_decoration_manager_v1, zxdg_toplevel_decoration_v1,
};
delegate_noop!(Client: ignore zxdg_decoration_manager_v1::ZxdgDecorationManagerV1);
impl Dispatch<zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1, usize> for Client {
    fn event(
        state: &mut Self,
        _: &zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1,
        event: zxdg_toplevel_decoration_v1::Event,
        id: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zxdg_toplevel_decoration_v1::Event::Configure {
            mode: wayland_client::WEnum::Value(mode),
        } = event
        {
            state.decoration_modes.insert(*id, mode as u32);
            *state.decoration_configure_counts.entry(*id).or_default() += 1;
        }
    }
}

#[test]
#[ignore = "requires private Unix sockets and an isolated XDG_RUNTIME_DIR"]
fn xdg_decoration_negotiation_and_geometry() {
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
        let manager: zxdg_decoration_manager_v1::ZxdgDecorationManagerV1 =
            globals.bind(&qh, 1..=1, ()).unwrap();
        let mut client = Client::default();
        let surface = compositor.create_surface(&qh, ());
        let xdg = wm.get_xdg_surface(&surface, &qh, ());
        let top = xdg.get_toplevel(&qh, 1);
        // A decoration object with no requested mode defaults to server-side.
        let decoration = manager.get_toplevel_decoration(&top, &qh, 1);
        top.set_app_id("arbitrary.application".into());
        surface.commit();
        queue.roundtrip(&mut client).unwrap();
        queue.roundtrip(&mut client).unwrap();
        let sibling_surface = compositor.create_surface(&qh, ());
        let sibling_xdg = wm.get_xdg_surface(&sibling_surface, &qh, ());
        let sibling_top = sibling_xdg.get_toplevel(&qh, 2);
        sibling_top.set_app_id("sibling.application".into());
        sibling_surface.commit();
        queue.roundtrip(&mut client).unwrap();
        queue.roundtrip(&mut client).unwrap();
        let sibling_configures = client.configure_counts[&2];
        assert_eq!(
            client.decoration_modes[&1],
            zxdg_toplevel_decoration_v1::Mode::ServerSide as u32
        );
        decoration.set_mode(zxdg_toplevel_decoration_v1::Mode::ClientSide);
        surface.commit();
        queue.roundtrip(&mut client).unwrap();
        queue.roundtrip(&mut client).unwrap();
        assert_eq!(
            client.decoration_modes[&1],
            zxdg_toplevel_decoration_v1::Mode::ClientSide as u32
        );
        assert_eq!(
            client.configure_counts[&2], sibling_configures,
            "changing one window's decoration mode must not configure its sibling"
        );
        let client_side_configures = client.decoration_configure_counts[&1];
        let client_side_toplevel_configures = client.configure_counts[&1];
        decoration.set_mode(zxdg_toplevel_decoration_v1::Mode::ClientSide);
        surface.commit();
        queue.roundtrip(&mut client).unwrap();
        queue.roundtrip(&mut client).unwrap();
        assert_eq!(
            client.configure_counts[&1],
            client_side_toplevel_configures + 1,
            "repeating set_mode must still receive an xdg_surface.configure response"
        );
        assert_eq!(
            client.decoration_configure_counts[&1], client_side_configures,
            "repeating the negotiated mode must not resend the decoration mode"
        );
        inspect(&sender, |state| {
            let (window, rect, fullscreen, _, _, _) = &state.workspace_layout(0)[0];
            assert!(crate::workspaces::titlebar_rect(window, *rect, *fullscreen).is_none());
        });
        decoration.set_mode(zxdg_toplevel_decoration_v1::Mode::ServerSide);
        surface.commit();
        queue.roundtrip(&mut client).unwrap();
        queue.roundtrip(&mut client).unwrap();
        assert_eq!(
            client.decoration_modes[&1],
            zxdg_toplevel_decoration_v1::Mode::ServerSide as u32
        );
        let server_side_configures = client.configure_counts[&1];
        decoration.unset_mode();
        surface.commit();
        queue.roundtrip(&mut client).unwrap();
        queue.roundtrip(&mut client).unwrap();
        assert_eq!(
            client.configure_counts[&1],
            server_side_configures + 1,
            "unset_mode must receive an xdg_surface.configure response even when unchanged"
        );
        assert_eq!(
            client.decoration_modes[&1],
            zxdg_toplevel_decoration_v1::Mode::ServerSide as u32
        );
        inspect(&sender, |state| {
            let layout = state.workspace_layout(0);
            let (window, rect, fullscreen, _, _, _) = &layout[0];
            let bar = crate::workspaces::titlebar_rect(window, *rect, *fullscreen).unwrap();
            assert_eq!(bar.size.h, 34);
            assert_eq!(bar.loc.y + bar.size.h, rect.loc.y);
            state.host_focused = true;
            state.pointer_location = (
                f64::from(bar.loc.x + bar.size.w - 36),
                f64::from(bar.loc.y + 17),
            )
                .into();
            assert!(state.handle_compositor_button(
                0x110,
                smithay::backend::input::ButtonState::Pressed,
                1
            ));
            assert!(state.workspaces[0].maximized.is_some());
            state.handle_compositor_button(
                0x110,
                smithay::backend::input::ButtonState::Released,
                2,
            );
        });

        decoration.destroy();
        surface.commit();
        queue.roundtrip(&mut client).unwrap();
        queue.roundtrip(&mut client).unwrap();
        inspect(&sender, |state| {
            let layout = state.workspace_layout(0);
            let (window, rect, fullscreen, _, _, _) = &layout[0];
            assert!(crate::workspaces::titlebar_rect(window, *rect, *fullscreen).is_none());
        });

        // Toplevels without the protocol object do not gain a compositor frame.
        let bare_surface = compositor.create_surface(&qh, ());
        let bare_xdg = wm.get_xdg_surface(&bare_surface, &qh, ());
        let bare_top = bare_xdg.get_toplevel(&qh, 2);
        bare_top.set_app_id("undecorated.application".into());
        bare_surface.commit();
        queue.roundtrip(&mut client).unwrap();
        queue.roundtrip(&mut client).unwrap();
        inspect(&sender, |state| {
            let bare = state.workspace_layout(0).into_iter().find(|(window, ..)| {
                window.toplevel().is_some_and(|top| {
                    smithay::wayland::compositor::with_states(top.wl_surface(), |states| {
                        states
                            .data_map
                            .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                            .and_then(|data| {
                                data.lock()
                                    .ok()
                                    .and_then(|attributes| attributes.app_id.clone())
                            })
                            .as_deref()
                            == Some("undecorated.application")
                    })
                })
            });
            let (window, rect, fullscreen, _, _, _) = bare.expect("bare toplevel mapped");
            assert!(crate::workspaces::titlebar_rect(&window, rect, fullscreen).is_none());
        });
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    while !thread.is_finished() {
        assert!(
            Instant::now() < deadline,
            "titlebar protocol test timed out"
        );
        event_loop
            .dispatch(Duration::from_millis(2), &mut state)
            .unwrap();
        while let Ok(inspect) = requests.try_recv() {
            inspect(&mut state);
        }
        state.space.refresh();
        state.display_handle.flush_clients().unwrap();
    }
    thread.join().unwrap();
}
