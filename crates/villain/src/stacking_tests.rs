//! Real Wayland surfaces prove that focus changes pixels and input stacking.
use super::drag_tests::{button, motion};
use super::*;
use smithay::backend::input::ButtonState::{Pressed, Released};

#[test]
#[ignore = "requires private Unix sockets, isolated XDG_RUNTIME_DIR and surfaceless GLES"]
fn floating_focus_raises_and_preserves_family_stacking() {
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
        let _keyboard = seat.get_keyboard(&qh, ());
        let mut client = Client::default();
        let path =
            std::env::temp_dir().join(format!("villain-stacking-buffer-{}", std::process::id()));
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
            top.set_app_id(format!("stack-{id}"));
            if id == 2 {
                let (_, _, parent) = &surfaces[0];
                top.set_parent(Some(parent));
            }
            surface.commit();
            surfaces.push((surface, xdg, top));
        }
        queue.roundtrip(&mut client).unwrap();
        queue.roundtrip(&mut client).unwrap();
        for (id, (surface, _, _)) in surfaces.iter().enumerate() {
            let (width, height) = match id {
                0 | 1 => (400, 300),
                2 => (180, 120),
                _ => (800, 600),
            };
            let buffer = pool.create_buffer(
                id as i32 * 800 * 600 * 4,
                width,
                height,
                width * 4,
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
            let window = |s: &Villain, name| s.capture_window(find(s, name).id).unwrap();
            let red = window(s, "stack-0");
            let green = window(s, "stack-1");
            let blue = window(s, "stack-2");
            let tile = window(s, "stack-3");
            for floating in [&red, &green] {
                s.focus_managed_window(floating);
                s.toggle_floating_focused().unwrap();
            }
            for (window, x, y, w, h) in [
                (&red, 100, 100, 400, 300),
                (&green, 250, 180, 400, 300),
                (&blue, 180, 130, 180, 120),
            ] {
                s.set_floating_geometry(window, Rectangle::new((x, y).into(), (w, h).into()));
            }
            let display = unsafe {
                smithay::backend::egl::EGLDisplay::new(
                    smithay::backend::egl::native::EGLSurfacelessDisplay,
                )
                .unwrap()
            };
            let context = smithay::backend::egl::EGLContext::new(&display).unwrap();
            let mut renderer =
                unsafe { smithay::backend::renderer::gles::GlesRenderer::new(context).unwrap() };
            let assert_top = |s: &Villain, renderer: &mut _, expected: &Window, color: [u8; 4]| {
                assert_eq!(s.focused_window().as_ref(), Some(expected));
                let pixels = super::tile_tests::scene_pixels(s, renderer);
                assert_eq!(&pixels[(350 * 800 + 400) * 4..(350 * 800 + 401) * 4], color);
                let layout = s.workspace_preview_scene(0).unwrap().windows;
                let position =
                    |window: &Window| layout.iter().position(|(w, _)| w == window).unwrap();
                assert!(position(&tile) < position(&red));
                assert!(
                    position(&red) < position(&blue),
                    "dialog remains above parent in preview"
                );
            };
            s.focus_managed_window(&green);
            assert_top(s, &mut renderer, &green, [0, 255, 0, 255]);
            s.focus_window(s.window_id(&red).unwrap());
            assert_top(s, &mut renderer, &red, [255, 0, 0, 255]);
            let pixels = super::tile_tests::scene_pixels(s, &mut renderer);
            assert_eq!(
                &pixels[(200 * 800 + 300) * 4..(200 * 800 + 301) * 4],
                [0, 0, 255, 255]
            );
            s.focus_managed_window(&green);
            s.config.input.focus_follows_mouse = true;
            motion(s, 150.0, 350.0);
            assert_top(s, &mut renderer, &red, [255, 0, 0, 255]);
            s.config.input.focus_follows_mouse = false;
            motion(s, 600.0, 400.0);
            assert_top(s, &mut renderer, &red, [255, 0, 0, 255]);
            button(s, 0x110, Pressed);
            button(s, 0x110, Released);
            assert_top(s, &mut renderer, &green, [0, 255, 0, 255]);
            s.pointer_location = (400.0, 350.0).into();
            s.refresh_pointer_surface(0);
            assert_eq!(s.input_window_at_pointer().unwrap().0, green);
            s.relayout_active_workspace();
            s.switch_workspace(1);
            s.switch_workspace(0);
            assert_top(s, &mut renderer, &green, [0, 255, 0, 255]);
            s.focus_managed_window(&tile);
            let pixels = super::tile_tests::scene_pixels(s, &mut renderer);
            assert_eq!(
                &pixels[(350 * 800 + 400) * 4..(350 * 800 + 401) * 4],
                [0, 255, 0, 255],
                "tile focus must not cover floats"
            );
            s.config.focus_cycle_order = knave_config::FocusCycleOrder::Mru;
            let history = s.workspaces[0].focus_history.clone();
            for _ in 0..4 {
                s.cycle_focus(crate::dispatch::FocusCycleStep::Next);
            }
            assert_eq!(s.focused_window(), Some(tile.clone()));
            assert_eq!(
                s.workspaces[0].focus_history, history,
                "raising must not rewrite MRU cycle order"
            );
            s.focus_managed_window(&green);
            assert!(s.minimize_focused_window());
            assert!(s.restore_last_minimized_window());
            s.focus_managed_window(&green);
            assert_top(s, &mut renderer, &green, [0, 255, 0, 255]);
        });
        // A dialog must also rise above its siblings, while staying above the parent.
        surfaces[1].2.set_parent(Some(&surfaces[0].2));
        surfaces[1].0.commit();
        queue.roundtrip(&mut client).unwrap();
        queue.roundtrip(&mut client).unwrap();
        inspect(&sender, |s| {
            let green = s.capture_window(find(s, "stack-1").id).unwrap();
            let blue = s.capture_window(find(s, "stack-2").id).unwrap();
            let display = unsafe {
                smithay::backend::egl::EGLDisplay::new(
                    smithay::backend::egl::native::EGLSurfacelessDisplay,
                )
                .unwrap()
            };
            let context = smithay::backend::egl::EGLContext::new(&display).unwrap();
            let mut renderer =
                unsafe { smithay::backend::renderer::gles::GlesRenderer::new(context).unwrap() };
            for (window, color) in [
                (&blue, [0, 0, 255, 255]),
                (&green, [0, 255, 0, 255]),
                (&blue, [0, 0, 255, 255]),
            ] {
                s.focus_managed_window(window);
                s.relayout_active_workspace();
                let pixels = super::tile_tests::scene_pixels(s, &mut renderer);
                assert_eq!(
                    &pixels[(200 * 800 + 300) * 4..(200 * 800 + 301) * 4],
                    color,
                    "focused sibling dialog is on top"
                );
            }
        });
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    while !thread.is_finished() {
        assert!(
            Instant::now() < deadline,
            "floating stacking test timed out"
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
