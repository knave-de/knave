//! Real surfaces, actual pointer grabs, rendered pixels and binding delivery.
use super::*;
use crate::keybinds::handle_keyboard_key;
use knave_desktop_api::{DesktopCommand, WindowTarget};
use smithay::{
    backend::input::{ButtonState, KeyState},
    input::keyboard::Keycode,
    utils::SERIAL_COUNTER,
};

pub(super) fn button(s: &mut Villain, code: u32, state: ButtonState) {
    if !s.handle_compositor_button(code, state, 1) {
        s.refresh_pointer_and_focus(1);
        s.pointer.clone().button(
            s,
            &smithay::input::pointer::ButtonEvent {
                serial: SERIAL_COUNTER.next_serial(),
                time: 1,
                button: code,
                state,
            },
        );
    }
}
pub(super) fn motion(s: &mut Villain, x: f64, y: f64) {
    s.pointer_location = (x, y).into();
    s.refresh_pointer_motion(1);
}
fn key(s: &mut Villain, code: u32, state: KeyState) {
    handle_keyboard_key(s, Keycode::new(code), state, 1);
}

#[test]
#[ignore = "requires private Unix sockets, isolated XDG_RUNTIME_DIR and surfaceless GLES"]
fn desktop_drag_resize_and_floating_lifecycle() {
    let mut event_loop = EventLoop::try_new().unwrap();
    let mut config = crate::config::RuntimeConfig::load().unwrap();
    config.appearance = Default::default();
    let mut state = Villain::new(&mut event_loop, Display::new().unwrap(), config);
    let (server, client) = UnixStream::pair().unwrap();
    state
        .display_handle
        .insert_client(server, Arc::new(crate::state::ClientState::default()))
        .unwrap();
    let (sender, requests) = mpsc::channel::<Inspect>();
    let thread = std::thread::spawn(move || {
        use std::io::Write;
        let conn = Connection::from_socket(client).unwrap();
        let (globals, mut queue) = registry_queue_init::<Client>(&conn).unwrap();
        let qh = queue.handle();
        let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=4, ()).unwrap();
        let wm: xdg_wm_base::XdgWmBase = globals.bind(&qh, 1..=6, ()).unwrap();
        let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).unwrap();
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ()).unwrap();
        let _pointer = seat.get_pointer(&qh, ());
        let mut client = Client::default();
        let path = std::env::temp_dir().join(format!("villain-drag-buffer-{}", std::process::id()));
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        std::fs::remove_file(path).unwrap();
        for color in [0xffff0000_u32, 0xff00ff00, 0xff0000ff, 0xffffff00] {
            file.write_all(&color.to_ne_bytes().repeat(800 * 600))
                .unwrap();
        }
        let pool = shm.create_pool(file.as_fd(), 800 * 600 * 4 * 4, &qh, ());
        let mut surfaces = Vec::new();
        for id in 0..4 {
            let surface = compositor.create_surface(&qh, ());
            let xdg = wm.get_xdg_surface(&surface, &qh, ());
            let top = xdg.get_toplevel(&qh, id);
            top.set_app_id(format!("drag-{id}"));
            surface.commit();
            surfaces.push((surface, xdg, top));
        }
        queue.roundtrip(&mut client).unwrap();
        queue.roundtrip(&mut client).unwrap();
        for (id, (surface, _, _)) in surfaces.iter().enumerate() {
            let buffer = pool.create_buffer(
                id as i32 * 800 * 600 * 4,
                800,
                600,
                800 * 4,
                wl_shm::Format::Argb8888,
                &qh,
                (),
            );
            surface.attach(Some(&buffer), 0, 0);
            surface.commit();
        }
        queue.roundtrip(&mut client).unwrap();
        queue.roundtrip(&mut client).unwrap();
        inspect(&sender, |s| {
            use ButtonState::{Pressed, Released};
            use KeyState::{Pressed as Down, Released as Up};
            let original = s.tiled_slots();
            assert_eq!(original.len(), 4);
            let source = original[0].0.clone();
            let target = original[1].0.clone();
            motion(s, 100.0, 100.0);
            key(s, 133, Down); // Super
            button(s, 0x110, Pressed);
            assert!(s.interaction.is_some());
            assert!(s.pending_modifier.is_none());
            motion(s, 600.0, 100.0);
            assert_eq!(s.tiled_slots(), original, "no reorder until drop");
            assert_eq!(s.focused_window(), Some(source.clone()));
            let display = unsafe {
                smithay::backend::egl::EGLDisplay::new(
                    smithay::backend::egl::native::EGLSurfacelessDisplay,
                )
                .unwrap()
            };
            let context = smithay::backend::egl::EGLContext::new(&display).unwrap();
            let mut renderer =
                unsafe { smithay::backend::renderer::gles::GlesRenderer::new(context).unwrap() };
            let pixels = super::tile_tests::scene_pixels(s, &mut renderer);
            let sample = |x: usize, y: usize| &pixels[(y * 800 + x) * 4..(y * 800 + x + 1) * 4];
            assert!(sample(700, 100)[0] > 150, "live red window follows pointer");
            assert!(sample(100, 100)[0] < 100, "original slot is a placeholder");
            button(s, 0x111, Pressed); // extra button must not prolong the drag
            button(s, 0x110, Released);
            assert!(s.interaction.is_none());
            assert_eq!(s.tiled_slots()[0].0, target);
            assert_eq!(s.tiled_slots()[1].0, source);
            button(s, 0x111, Released);
            key(s, 133, Up);
            assert!(!s.overview_visible, "MOD drag must not toggle overview");
            button(s, 0x110, Pressed);
            button(s, 0x110, Released);
            assert!(
                !s.pointer.is_grabbed(),
                "extra drag button cannot leave the next normal click grabbed"
            );

            // Modal IPC drag and Escape use the same lifecycle.
            let before = s.tiled_slots();
            motion(s, 100.0, 100.0);
            assert!(matches!(
                s.dispatch(
                    DesktopCommand::MoveWindow {
                        target: WindowTarget::Pointer
                    }
                    .into()
                ),
                Ok(())
            ));
            motion(s, 600.0, 500.0);
            key(s, 9, Down);
            key(s, 9, Up); // Escape
            assert_eq!(s.tiled_slots(), before);
            assert!(!s.pointer.is_grabbed());
            s.begin_window_interaction(WindowTarget::Pointer, false, None)
                .unwrap();
            motion(s, 100.0, 100.0);
            key(s, 36, Down);
            key(s, 36, Up); // Enter commits
            assert_ne!(s.tiled_slots(), before);

            // Adjacent stack resizing preserves the third stack window.
            let before = s.tiled_slots();
            motion(s, 600.0, 200.0);
            button(s, 0x110, Pressed);
            assert!(s.split_drag_active);
            motion(s, 600.0, 260.0);
            let resized = s.tiled_slots();
            assert_eq!(resized[1].1.size.h, 260);
            assert_eq!(resized[2].1.size.h, 140);
            assert_eq!(resized[3], before[3]);
            button(s, 0x110, Released);
            let weights = s.stack_weights();
            motion(s, 600.0, 260.0);
            button(s, 0x110, Pressed);
            motion(s, 600.0, 320.0);
            key(s, 9, Down);
            key(s, 9, Up);
            button(s, 0x110, Released);
            assert_eq!(s.stack_weights(), weights);
            s.switch_workspace(1);
            s.switch_workspace(0);
            assert_eq!(s.stack_weights(), weights);

            // MOD+V creates a user override that survives client commits.
            let floating = s.tiled_slots()[0].0.clone();
            let id = s.window_id(&floating).unwrap();
            s.focus_window(id);
            key(s, 133, Down);
            key(s, 55, Down);
            key(s, 55, Up);
            key(s, 133, Up);
            let rect = s.floating_geometry(&floating).unwrap();
            s.refresh_window_hints(&floating);
            assert_eq!(s.floating_geometry(&floating), Some(rect));
            motion(
                s,
                f64::from(rect.loc.x + rect.size.w + 1),
                f64::from(rect.loc.y + 50),
            );
            button(s, 0x110, Pressed);
            assert!(
                s.interaction.is_some(),
                "unmodified outer edge starts resize"
            );
            motion(
                s,
                f64::from(rect.loc.x + rect.size.w + 41),
                f64::from(rect.loc.y + 50),
            );
            button(s, 0x110, Released);
            let resized = s.floating_geometry(&floating).unwrap();
            assert_eq!(resized.size.w, rect.size.w + 40);
            s.focus_window(id);
            s.toggle_floating_focused().unwrap();
            assert!(s.floating_geometry(&floating).is_none());
            s.toggle_floating_focused().unwrap();
            assert_eq!(s.floating_geometry(&floating), Some(resized));
            s.pointer_location = (200.0, 200.0).into();
            s.begin_window_interaction(WindowTarget::Window(id), false, None)
                .unwrap();
            motion(s, 230.0, 240.0);
            assert_ne!(s.floating_geometry(&floating), Some(resized));
            s.release_pointer_buttons();
            assert_eq!(s.floating_geometry(&floating), Some(resized));
            assert!(s.interaction.is_none() && !s.pointer.is_grabbed());
            s.begin_window_interaction(WindowTarget::Window(id), false, None)
                .unwrap();
            button(s, 0x110, Pressed);
            key(s, 9, Down);
            key(s, 9, Up);
            button(s, 0x110, Released);
            motion(
                s,
                f64::from(resized.loc.x + 50),
                f64::from(resized.loc.y + 50),
            );
            button(s, 0x110, Pressed);
            button(s, 0x110, Released);
            assert!(
                !s.pointer.is_grabbed(),
                "Escape with a held button cannot leave a later click grabbed"
            );
            s.set_window_maximized(&floating, true);
            assert!(
                s.begin_window_interaction(WindowTarget::Window(id), false, None)
                    .is_err()
            );
            s.set_window_maximized(&floating, false);
            s.begin_window_interaction(WindowTarget::Window(id), true, None)
                .unwrap();
            motion(s, 400.0, 400.0);
            s.minimize_focused_window();
            assert!(s.interaction.is_none() && !s.pointer.is_grabbed());
            // User-tiled transient children disappear with a minimized parent.
            // Hidden children cannot reserve slots or expose a phantom divider.
            for entry in &mut s.workspaces[s.active_workspace].windows {
                if entry.id != id {
                    entry.parent = Some(id);
                }
            }
            s.relayout_active_workspace();
            assert!(s.tiled_slots().is_empty());
            motion(s, 600.0, 200.0);
            assert!(s.stack_divider_under_pointer().is_none());
        });
        queue.roundtrip(&mut client).unwrap();
        queue.roundtrip(&mut client).unwrap();
        assert_eq!(
            client.pointer_buttons, 4,
            "compositor gestures must not leak buttons"
        );
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    while !thread.is_finished() {
        assert!(Instant::now() < deadline, "drag test timed out");
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
