//! Oversized buffers must not paint or receive input outside assigned tiles.
use super::*;
use smithay::backend::{
    allocator::Fourcc,
    egl::{EGLContext, EGLDisplay, native::EGLSurfacelessDisplay},
    renderer::{
        Bind, ExportMem, Offscreen,
        damage::OutputDamageTracker,
        gles::{GlesRenderer, GlesTexture},
    },
};
use wayland_client::protocol::{wl_subcompositor, wl_subsurface};
use wayland_protocols::xdg::shell::client::{xdg_popup, xdg_positioner};

delegate_noop!(Client: ignore xdg_popup::XdgPopup);
delegate_noop!(Client: ignore xdg_positioner::XdgPositioner);
delegate_noop!(Client: ignore wl_subcompositor::WlSubcompositor);
delegate_noop!(Client: ignore wl_subsurface::WlSubsurface);

pub(super) fn scene_pixels(state: &Villain, renderer: &mut GlesRenderer) -> Vec<u8> {
    use crate::appearance::render::{EffectsRenderer, SceneLayers};
    use smithay::utils::{Rectangle, Transform};
    let elements = EffectsRenderer::default()
        .scene(state, renderer, 0, SceneLayers::All, 1.0)
        .unwrap();
    let mut texture: GlesTexture = renderer
        .create_buffer(Fourcc::Abgr8888, (800, 600).into())
        .unwrap();
    let mut framebuffer = renderer.bind(&mut texture).unwrap();
    let mut tracker = OutputDamageTracker::new((800, 600), 1.0, Transform::Normal);
    tracker
        .render_output(
            renderer,
            &mut framebuffer,
            0,
            &elements,
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap()
        .sync
        .wait()
        .unwrap();
    let mapping = renderer
        .copy_framebuffer(
            &framebuffer,
            Rectangle::from_size((800, 600).into()),
            Fourcc::Abgr8888,
        )
        .unwrap();
    renderer.map_texture(&mapping).unwrap().to_vec()
}

#[test]
#[ignore = "requires private Unix sockets, isolated XDG_RUNTIME_DIR and surfaceless GLES"]
fn oversized_tiles_clip_pixels_and_input_but_preserve_popups() {
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
        let subcompositor: wl_subcompositor::WlSubcompositor =
            globals.bind(&qh, 1..=1, ()).unwrap();
        let wm: xdg_wm_base::XdgWmBase = globals.bind(&qh, 1..=6, ()).unwrap();
        let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).unwrap();
        let mut client = Client::default();
        let settle = |queue: &mut wayland_client::EventQueue<Client>, client: &mut Client| {
            queue.roundtrip(client).unwrap();
            queue.roundtrip(client).unwrap();
        };
        let create = |id, name: &str| {
            let surface = compositor.create_surface(&qh, ());
            let xdg = wm.get_xdg_surface(&surface, &qh, ());
            let top = xdg.get_toplevel(&qh, id);
            top.set_app_id(name.into());
            surface.commit();
            (surface, xdg, top)
        };
        let (a, ax, _at) = create(1, "master");
        let (b, bx, _bt) = create(2, "oversized");
        let (c, cx, _ct) = create(3, "transparent");
        settle(&mut queue, &mut client);
        assert_eq!(
            (client.configures[&2].width, client.configures[&2].height),
            (400, 300)
        );
        // The green client ignores its 400x300 configure. An offset window
        // geometry and an outlying subsurface also exercise coordinate mapping.
        let path = std::env::temp_dir().join(format!("villain-tile-buffer-{}", std::process::id()));
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        std::fs::remove_file(path).unwrap();
        for color in [0xffff0000_u32, 0xff00ff00, 0x80000080, 0xffffff00] {
            file.write_all(&color.to_ne_bytes().repeat(800 * 600))
                .unwrap();
        }
        let pool = shm.create_pool(file.as_fd(), 800 * 600 * 4 * 4, &qh, ());
        let buffer = |index, width, height| {
            pool.create_buffer(
                index * 800 * 600 * 4,
                width,
                height,
                width * 4,
                wl_shm::Format::Argb8888,
                &qh,
                (),
            )
        };
        let red = buffer(0, 800, 600);
        let green = buffer(1, 800, 600);
        let blue = buffer(2, 400, 300);
        let yellow = buffer(3, 80, 40);
        a.attach(Some(&red), 0, 0);
        ax.set_window_geometry(10, 20, 780, 580);
        a.commit();
        b.attach(Some(&green), 0, 0);
        bx.set_window_geometry(10, 20, 780, 580);
        let sub = compositor.create_surface(&qh, ());
        let subsurface = subcompositor.get_subsurface(&sub, &b, &qh, ());
        subsurface.set_position(-100, 350);
        sub.attach(Some(&green), 0, 0);
        sub.commit();
        b.commit();
        c.attach(Some(&blue), 0, 0);
        c.commit();
        settle(&mut queue, &mut client);

        let menu = compositor.create_surface(&qh, ());
        let menu_xdg = wm.get_xdg_surface(&menu, &qh, ());
        let positioner = wm.create_positioner(&qh, ());
        positioner.set_size(80, 40);
        positioner.set_anchor_rect(0, 50, 1, 1);
        positioner.set_anchor(xdg_positioner::Anchor::TopLeft);
        positioner.set_gravity(xdg_positioner::Gravity::BottomRight);
        positioner.set_offset(-100, 0);
        let _popup = menu_xdg.get_popup(Some(&cx), &positioner, &qh, ());
        menu.commit();
        settle(&mut queue, &mut client);
        menu.attach(Some(&yellow), 0, 0);
        menu.commit();
        settle(&mut queue, &mut client);
        inspect(&sender, |s| {
            // Surfaceless EGL remains on the server thread throughout rendering.
            let display = unsafe { EGLDisplay::new(EGLSurfacelessDisplay).unwrap() };
            let context = EGLContext::new(&display).unwrap();
            let mut renderer = unsafe { GlesRenderer::new(context).unwrap() };
            for mode in 0..3 {
                s.config.appearance = Default::default();
                if mode == 1 {
                    // Enter effects composition without a rounded/opacity mask.
                    s.config.appearance.border.width.top = 2;
                } else if mode == 2 {
                    s.config.appearance.border.radius.top_left = 12;
                }
                s.relayout_active_workspace();
                let pixels = scene_pixels(s, &mut renderer);
                let sample = |x: usize, y: usize| &pixels[(y * 800 + x) * 4..(y * 800 + x + 1) * 4];
                assert_eq!(
                    sample(600, 150),
                    [0, 255, 0, 255],
                    "green tile, mode {mode}"
                );
                assert_eq!(
                    sample(600, 450),
                    [0, 0, 128, 255],
                    "green must not leak behind transparent blue, mode {mode}"
                );
                assert_eq!(
                    sample(350, 500),
                    [255, 0, 0, 255],
                    "subsurface must stay in its tile, mode {mode}"
                );
                assert_eq!(
                    sample(320, 370),
                    [255, 255, 0, 255],
                    "popup outside parent must remain visible, mode {mode}"
                );
                let layout = s.workspace_layout(0);
                // Raise oversized green for hit testing so stacking cannot hide a bad hit.
                s.space.raise_element(&layout[1].0, false);
                s.pointer_location = (600.0, 450.0).into();
                let hit = s.input_window_at_pointer().unwrap().0;
                assert_eq!(
                    hit, layout[2].0,
                    "overflow cannot intercept neighboring input"
                );
                s.pointer_location = (350.0, 500.0).into();
                assert_eq!(s.input_window_at_pointer().unwrap().0, layout[0].0);
                s.pointer_location = (320.0, 370.0).into();
                assert_eq!(
                    s.input_window_at_pointer().unwrap().0,
                    layout[2].0,
                    "popup input remains outside tile"
                );
            }
        });
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    while !thread.is_finished() {
        assert!(Instant::now() < deadline, "tile clipping test timed out");
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
