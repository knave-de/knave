//! Exercise layer-shell over a real private Wayland connection.
use super::*;
use smithay::reexports::{calloop::EventLoop, wayland_server::Display};
use std::{
    os::{fd::AsFd, unix::net::UnixStream},
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};
use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface},
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1 as shell, zwlr_layer_surface_v1 as layer,
};

#[derive(Default)]
struct Client {
    sizes: Vec<(u32, u32)>,
    app_size: (i32, i32),
    frames: usize,
}
impl Dispatch<wayland_client::protocol::wl_callback::WlCallback, ()> for Client {
    fn event(
        state: &mut Self,
        _: &wayland_client::protocol::wl_callback::WlCallback,
        _: wayland_client::protocol::wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.frames += 1;
    }
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
impl Dispatch<layer::ZwlrLayerSurfaceV1, ()> for Client {
    fn event(
        state: &mut Self,
        layer: &layer::ZwlrLayerSurfaceV1,
        event: layer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let layer::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            state.sizes.push((width, height));
            layer.ack_configure(serial);
        }
    }
}
use wayland_protocols::xdg::shell::client::{
    xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base,
};
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
impl Dispatch<xdg_toplevel::XdgToplevel, ()> for Client {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Configure { width, height, .. } = event {
            state.app_size = (width, height);
        }
    }
}
delegate_noop!(Client: ignore xdg_popup::XdgPopup);
delegate_noop!(Client: ignore xdg_positioner::XdgPositioner);
delegate_noop!(Client: ignore wayland_client::protocol::wl_seat::WlSeat);
delegate_noop!(Client: ignore wl_compositor::WlCompositor);
delegate_noop!(Client: ignore wl_surface::WlSurface);
delegate_noop!(Client: ignore wl_shm::WlShm);
delegate_noop!(Client: ignore wl_shm_pool::WlShmPool);
delegate_noop!(Client: ignore wl_buffer::WlBuffer);
delegate_noop!(Client: ignore shell::ZwlrLayerShellV1);
type Inspect = Box<dyn FnOnce(&mut Villain) + Send>;
fn inspect(sender: &mpsc::Sender<Inspect>, f: impl FnOnce(&mut Villain) + Send + 'static) {
    let (done, recv) = mpsc::channel();
    sender
        .send(Box::new(move |s| {
            f(s);
            done.send(()).unwrap();
        }))
        .unwrap();
    recv.recv_timeout(Duration::from_secs(5)).unwrap();
}
#[test]
#[ignore = "requires private Unix sockets and an isolated XDG_RUNTIME_DIR"]
fn layer_lifecycle_focus_and_workspace_independence() {
    let mut event_loop = EventLoop::try_new().unwrap();
    let mut state = Villain::new(
        &mut event_loop,
        Display::new().unwrap(),
        crate::config::RuntimeConfig::load().unwrap(),
    );
    let output = Output::new(
        "test".into(),
        smithay::output::PhysicalProperties {
            size: (0, 0).into(),
            subpixel: smithay::output::Subpixel::Unknown,
            make: "test".into(),
            model: "test".into(),
        },
    );
    output.change_current_state(
        Some(smithay::output::Mode {
            size: (800, 600).into(),
            refresh: 60_000,
        }),
        None,
        None,
        Some((0, 0).into()),
    );
    output.create_global::<Villain>(&state.display_handle);
    state.space.map_output(&output, (0, 0));
    layer_map_for_output(&output).arrange();
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
        let shell: shell::ZwlrLayerShellV1 = globals.bind(&qh, 1..=4, ()).unwrap();
        let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).unwrap();
        let mut client = Client::default();
        let settle = |q: &mut wayland_client::EventQueue<Client>, c: &mut Client| {
            q.roundtrip(c).unwrap();
            q.roundtrip(c).unwrap();
        };
        let wm: xdg_wm_base::XdgWmBase = globals.bind(&qh, 1..=6, ()).unwrap();
        let app = compositor.create_surface(&qh, ());
        let xdg = wm.get_xdg_surface(&app, &qh, ());
        let top = xdg.get_toplevel(&qh, ());
        app.commit();
        settle(&mut queue, &mut client);
        assert_eq!(client.app_size, (800, 600));
        let surface = compositor.create_surface(&qh, ());
        let layer =
            shell.get_layer_surface(&surface, None, shell::Layer::Top, "panel".into(), &qh, ());
        layer.set_size(0, 40);
        layer.set_anchor(layer::Anchor::Top | layer::Anchor::Left | layer::Anchor::Right);
        layer.set_exclusive_zone(40);
        settle(&mut queue, &mut client);
        assert!(
            client.sizes.is_empty(),
            "configure must wait for initial commit"
        );
        surface.commit();
        settle(&mut queue, &mut client);
        assert_eq!(client.sizes.last(), Some(&(800, 40)));
        inspect(&sender, |s| {
            assert!(!s.shell_surfaces[0].mapped);
            assert_eq!(s.usable_area().size.h, 600);
        });
        let path =
            std::env::temp_dir().join(format!("villain-layer-buffer-{}", std::process::id()));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        std::fs::remove_file(&path).unwrap();
        file.set_len(800 * 600 * 4).unwrap();
        let pool = shm.create_pool(file.as_fd(), 800 * 600 * 4, &qh, ());
        let buffer = pool.create_buffer(0, 800, 40, 800 * 4, wl_shm::Format::Argb8888, &qh, ());
        surface.attach(Some(&buffer), 0, 0);
        surface.commit();
        settle(&mut queue, &mut client);
        inspect(&sender, |s| {
            assert!(s.shell_surfaces[0].mapped);
            assert!(
                !s.workspace_has_fullscreen(s.active_workspace),
                "the initial workspace must not enter fullscreen"
            );
            assert!(
                s.layer_surface_visible(s.shell_surfaces[0].layer.wl_surface()),
                "top-layer panels remain visible before a window enters fullscreen"
            );
            assert_eq!(
                s.usable_area(),
                Rectangle::new((0, 40).into(), (800, 560).into())
            );
            s.pointer_location = (10.0, 10.0).into();
            s.refresh_pointer(0);
            assert_eq!(
                s.pointer.current_focus().as_ref(),
                Some(s.shell_surfaces[0].layer.wl_surface())
            );
            assert!(
                s.keyboard.current_focus()
                    != Some(KeyboardFocus::Wayland(
                        s.shell_surfaces[0].layer.wl_surface().clone()
                    )),
                "noninteractive panel must not take keyboard focus"
            );
            s.repaint_needed = false;
            s.switch_workspace(4);
            assert!(s.repaint_needed, "empty workspace transitions must repaint");
            assert!(s.shell_surfaces[0].mapped);
            assert_eq!(s.window_info().len(), 1);
        });
        inspect(&sender, |s| s.switch_workspace(0));
        settle(&mut queue, &mut client);
        assert_eq!(client.app_size, (800, 560));
        app.attach(Some(&buffer), 0, 0);
        app.frame(&qh, ());
        app.commit();
        settle(&mut queue, &mut client);
        inspect(&sender, |s| {
            let output = s.space.outputs().next().unwrap().clone();
            for window in s.space.elements() {
                Villain::send_window_frame(window, &output, s.start_time.elapsed());
            }
        });
        settle(&mut queue, &mut client);
        std::thread::sleep(Duration::from_secs(1));
        top.set_maximized();
        settle(&mut queue, &mut client);
        assert_eq!(client.app_size, (800, 560));
        inspect(&sender, |s| {
            let info = &s.window_info()[0];
            assert!(info.maximized && !info.fullscreen);
            assert_eq!(
                s.space
                    .element_location(s.space.elements().next().unwrap())
                    .unwrap(),
                (0, 40).into()
            );
        });
        layer.set_exclusive_zone(60);
        surface.commit();
        settle(&mut queue, &mut client);
        assert_eq!(client.app_size, (800, 540));
        layer.set_exclusive_zone(40);
        surface.commit();
        settle(&mut queue, &mut client);
        // The installed shell may still label its status bar as an overlay.
        // Fullscreen visibility is keyed to the stable bar namespace as well
        // as the layer so an older shell cannot cover the application.
        let overlay_bar_surface = compositor.create_surface(&qh, ());
        let overlay_bar = shell.get_layer_surface(
            &overlay_bar_surface,
            None,
            shell::Layer::Overlay,
            "knave-shell-bar".into(),
            &qh,
            (),
        );
        overlay_bar.set_size(0, 40);
        overlay_bar.set_anchor(layer::Anchor::Top | layer::Anchor::Left | layer::Anchor::Right);
        overlay_bar.set_exclusive_zone(0);
        settle(&mut queue, &mut client);
        overlay_bar_surface.commit();
        settle(&mut queue, &mut client);
        overlay_bar_surface.attach(Some(&buffer), 0, 0);
        overlay_bar_surface.commit();
        settle(&mut queue, &mut client);
        inspect(&sender, |s| {
            assert!(s.shell_surfaces[1].mapped);
            assert!(s.layer_surface_visible(s.shell_surfaces[1].layer.wl_surface()));
        });
        top.set_fullscreen(None);
        settle(&mut queue, &mut client);
        assert_eq!(client.app_size, (800, 600));
        inspect(&sender, |s| {
            assert!(
                !s.layer_surface_visible(s.shell_surfaces[0].layer.wl_surface()),
                "top-layer panels must be hidden while a fullscreen window is active"
            );
            assert!(
                s.layer_under_pointer(true).is_none(),
                "hidden panels must not receive pointer input"
            );
            assert!(
                !s.layer_surface_visible(s.shell_surfaces[1].layer.wl_surface()),
                "the Knave bar must be hidden during fullscreen even if it uses the overlay layer"
            );
            s.pointer_location = (10.0, 10.0).into();
            s.refresh_pointer_and_focus(0);
            assert!(
                s.window_info()[0].focused,
                "fullscreen application must retain focus under a hidden panel"
            );
            assert_ne!(
                s.pointer.current_focus().as_ref(),
                Some(s.shell_surfaces[0].layer.wl_surface()),
                "pointer input must go to the fullscreen application"
            );
        });
        top.unset_fullscreen();
        settle(&mut queue, &mut client);
        assert_eq!(client.app_size, (800, 560));
        inspect(&sender, |s| {
            assert!(s.window_info()[0].maximized);
            assert!(s.layer_surface_visible(s.shell_surfaces[0].layer.wl_surface()));
            assert!(s.layer_surface_visible(s.shell_surfaces[1].layer.wl_surface()));
        });
        overlay_bar.destroy();
        overlay_bar_surface.destroy();
        settle(&mut queue, &mut client);
        top.unset_maximized();
        settle(&mut queue, &mut client);
        inspect(&sender, |s| assert!(!s.window_info()[0].maximized));
        let other = compositor.create_surface(&qh, ());
        let other_xdg = wm.get_xdg_surface(&other, &qh, ());
        let other_top = other_xdg.get_toplevel(&qh, ());
        other.commit();
        settle(&mut queue, &mut client);
        let app_buffer =
            pool.create_buffer(0, 400, 600, 400 * 4, wl_shm::Format::Argb8888, &qh, ());
        app.attach(Some(&app_buffer), 0, 0);
        app.commit();
        other.attach(Some(&app_buffer), 0, 0);
        other.commit();
        settle(&mut queue, &mut client);
        for kind in [shell::Layer::Top, shell::Layer::Overlay] {
            for zone in [0, -1] {
                layer.set_layer(kind);
                layer.set_exclusive_zone(zone);
                surface.commit();
                settle(&mut queue, &mut client);
                inspect(&sender, |s| {
                    let expected = s.keyboard.current_focus();
                    assert!(expected.is_some());
                    s.pointer_location = (10.0, 10.0).into();
                    let obscured = s.space.element_under(s.pointer_location).unwrap().0;
                    assert_ne!(KeyboardFocus::for_window(obscured), expected);
                    s.refresh_pointer_and_focus(0);
                    assert_eq!(
                        s.pointer.current_focus().as_ref(),
                        Some(s.shell_surfaces[0].layer.wl_surface())
                    );
                    assert_eq!(
                        s.keyboard.current_focus(),
                        expected,
                        "overlapping noninteractive panels must preserve focus"
                    );
                    s.keyboard.clone().set_focus(
                        s,
                        None,
                        smithay::utils::SERIAL_COUNTER.next_serial(),
                    );
                    s.refresh_pointer_and_focus(0);
                    assert_eq!(
                        s.keyboard.current_focus(),
                        expected,
                        "panel clicks recover MRU focus"
                    );
                });
            }
        }
        other_top.destroy();
        other_xdg.destroy();
        other.destroy();
        layer.set_layer(shell::Layer::Top);
        layer.set_exclusive_zone(40);
        surface.commit();
        settle(&mut queue, &mut client);
        layer.set_keyboard_interactivity(layer::KeyboardInteractivity::Exclusive);
        surface.commit();
        settle(&mut queue, &mut client);
        inspect(&sender, |s| {
            let expected = s.exclusive_layer_focus();
            assert!(expected.is_some());
            s.pointer_location = (500.0, 500.0).into();
            s.refresh_pointer_and_focus(0);
            assert_eq!(s.keyboard.current_focus(), expected);
            s.switch_workspace(1);
            assert_eq!(s.keyboard.current_focus(), expected);
            s.host_focused = false;
            s.refresh_pointer(0);
            assert!(s.keyboard.current_focus().is_none());
            s.host_focused = true;
            s.refresh_pointer(0);
            assert_eq!(s.keyboard.current_focus(), expected);
        });
        let seat: wayland_client::protocol::wl_seat::WlSeat = globals.bind(&qh, 1..=1, ()).unwrap();
        inspect(&sender, |s| {
            s.pointer_location = (10.0, 10.0).into();
            s.refresh_pointer(0);
            s.pressed_buttons.insert(272);
            s.pointer.clone().button(
                s,
                &smithay::input::pointer::ButtonEvent {
                    serial: smithay::utils::Serial::from(900),
                    time: 0,
                    button: 272,
                    state: smithay::backend::input::ButtonState::Pressed,
                },
            );
        });
        // A layer popup must be hit-testable outside its parent's rectangle.
        let menu = compositor.create_surface(&qh, ());
        let menu_xdg = wm.get_xdg_surface(&menu, &qh, ());
        let positioner = wm.create_positioner(&qh, ());
        positioner.set_size(100, 40);
        positioner.set_anchor_rect(20, 40, 1, 1);
        positioner.set_anchor(xdg_positioner::Anchor::BottomLeft);
        positioner.set_gravity(xdg_positioner::Gravity::BottomRight);
        let popup = menu_xdg.get_popup(None, &positioner, &qh, ());
        layer.get_popup(&popup);
        popup.grab(&seat, 900);
        menu.commit();
        settle(&mut queue, &mut client);
        let menu_buffer = pool.create_buffer(0, 100, 40, 400, wl_shm::Format::Argb8888, &qh, ());
        menu.attach(Some(&menu_buffer), 0, 0);
        menu.commit();
        settle(&mut queue, &mut client);
        inspect(&sender, |s| {
            s.pointer_location = (25.0, 45.0).into();
            s.refresh_pointer(0);
            assert!(s.keyboard.is_grabbed());
            s.switch_workspace(3);
            assert!(s.pointer.is_grabbed());
            let hit = s.pointer.current_focus().unwrap();
            assert_ne!(&hit, s.shell_surfaces[0].layer.wl_surface());
            assert!(s.layer_surface_visible(&hit));
        });
        surface.attach(None, 0, 0);
        surface.commit();
        settle(&mut queue, &mut client);
        inspect(&sender, |s| {
            assert!(!s.shell_surfaces[0].mapped);
            assert_eq!(s.usable_area().size.h, 600);
            assert!(s.keyboard.current_focus().is_none());
            assert!(!s.keyboard.is_grabbed());
            assert!(!s.pointer.is_grabbed());
        });
        popup.destroy();
        menu_xdg.destroy();
        menu.destroy();
        settle(&mut queue, &mut client);
        // Re-map requires a fresh initial commit/configure handshake.
        layer.set_size(0, 40);
        layer.set_anchor(layer::Anchor::Top | layer::Anchor::Left | layer::Anchor::Right);
        layer.set_exclusive_zone(40);
        surface.commit();
        settle(&mut queue, &mut client);
        surface.attach(Some(&buffer), 0, 0);
        surface.commit();
        settle(&mut queue, &mut client);
        inspect(&sender, |s| {
            assert!(s.shell_surfaces[0].mapped);
            assert_eq!(s.usable_area().size.h, 560);
        });
        let overview = compositor.create_surface(&qh, ());
        let overview_layer = shell.get_layer_surface(
            &overview,
            None,
            shell::Layer::Overlay,
            "knave-shell-overview".into(),
            &qh,
            (),
        );
        for late_hide in [false, true] {
            inspect(&sender, |s| {
                s.dispatch(crate::dispatch::Dispatch::SetOverviewVisible(true))
                    .unwrap();
            });
            overview_layer.set_size(800, 40);
            overview.commit();
            settle(&mut queue, &mut client);
            inspect(&sender, |s| {
                assert!(s.overview_visible);
                assert!(!s.overview_mapped());
                assert!(s.overview_panes.is_empty());
                s.pending_overview_panes = Some(vec![knave_desktop_api::OverviewPane {
                    workspace: knave_desktop_api::WorkspaceId(1),
                    x: 10,
                    y: 10,
                    width: 100,
                    height: 60,
                }]);
                assert!(
                    s.overview_panes.is_empty(),
                    "staged panes must not appear before the shell buffer"
                );
            });
            overview.attach(Some(&buffer), 0, 0);
            overview.commit();
            settle(&mut queue, &mut client);
            inspect(&sender, move |s| {
                assert!(s.shell_surfaces[1].mapped);
                assert!(s.overview_mapped());
                assert_eq!(s.overview_panes.len(), 1);
                assert!(s.pending_overview_panes.is_none());
                if late_hide {
                    s.dispatch(crate::dispatch::Dispatch::SetOverviewVisible(false))
                        .unwrap();
                    s.dispatch(crate::dispatch::Dispatch::SetOverviewVisible(true))
                        .unwrap();
                }
            });
            overview.attach(None, 0, 0);
            overview.commit();
            settle(&mut queue, &mut client);
            inspect(&sender, move |s| {
                assert!(!s.shell_surfaces[1].mapped);
                assert_eq!(s.overview_visible, late_hide);
                assert!(!s.shell_surfaces[1].hide_pending);
                assert!(!s.overview_mapped());
                assert!(s.overview_panes.is_empty());
                assert!(s.pending_overview_panes.is_none());
                s.publish_desktop_state();
            });
        }
        overview_layer.destroy();
        overview.destroy();
        settle(&mut queue, &mut client);
        inspect(&sender, |s| assert!(!s.overview_visible));
        inspect(&sender, |s| {
            s.switch_workspace(0);
            s.repaint_needed = false;
        });
        top.destroy();
        xdg.destroy();
        app.destroy();
        settle(&mut queue, &mut client);
        inspect(&sender, |s| {
            assert!(s.window_info().is_empty());
            assert!(s.repaint_needed, "closing the last window must repaint");
            s.pointer_location = (10.0, 10.0).into();
            s.refresh_pointer(0);
            assert_eq!(
                s.pointer.current_focus().as_ref(),
                Some(s.shell_surfaces[0].layer.wl_surface())
            );
        });
        let frames_before = client.frames;
        surface.frame(&qh, ());
        surface.commit();
        settle(&mut queue, &mut client);
        inspect(&sender, |s| {
            let output = s.space.outputs().next().unwrap().clone();
            s.schedule_frame_callbacks(&output);
        });
        while client.frames == frames_before {
            queue.blocking_dispatch(&mut client).unwrap();
        }
        layer.destroy();
        surface.destroy();
        settle(&mut queue, &mut client);
        inspect(&sender, |s| {
            assert!(s.shell_surfaces.is_empty());
            assert_eq!(s.usable_area().size.h, 600);
            s.switch_workspace(0);
            s.pointer_location = (2000.0, 100.0).into();
            s.refresh_pointer_and_focus(0);
            assert!(s.keyboard.current_focus().is_none());
        });
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    while !thread.is_finished() {
        assert!(Instant::now() < deadline);
        event_loop
            .dispatch(Duration::from_millis(2), &mut state)
            .unwrap();
        while let Ok(f) = requests.try_recv() {
            f(&mut state);
        }
        state.space.refresh();
        state.display_handle.flush_clients().unwrap();
    }
    thread.join().unwrap();
}
