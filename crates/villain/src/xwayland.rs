//! XWayland server and X11 window-manager integration.

use std::{os::fd::OwnedFd, process::Stdio};

use knave_desktop_api::XWaylandStatus;
use smithay::{
    desktop::Window,
    reexports::calloop::EventLoop,
    utils::{Logical, Rectangle},
    wayland::{
        selection::{
            SelectionTarget,
            data_device::{
                clear_data_device_selection, current_data_device_selection_userdata,
                request_data_device_client_selection, set_data_device_selection,
            },
            primary_selection::{
                clear_primary_selection, current_primary_selection_userdata,
                request_primary_client_selection, set_primary_selection,
            },
        },
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::{
        X11Surface, X11Wm, XWayland, XWaylandEvent, XwmHandler,
        xwm::{Reorder, ResizeEdge, WmWindowProperty, X11Window, XwmId},
    },
};

use crate::{state::Villain, workspaces::WindowAction};

pub struct UnmanagedWindow {
    pub window: Window,
    pub workspace: usize,
    pub parent: Option<Window>,
}

impl Villain {
    /// XWayland routes pointer events using its own X11 stacking order.
    /// Keep visible windows above windows retained on other workspaces.
    pub fn sync_x11_stacking(&mut self) {
        let Some(xwm) = self.xwm.as_mut() else {
            return;
        };
        for surface in self.space.elements().filter_map(Window::x11_surface) {
            if let Err(error) = xwm.raise_window(surface) {
                tracing::warn!(%error, "could not synchronize X11 stacking");
            }
        }
    }

    pub fn refresh_unmanaged_x11_windows(&mut self) {
        for entry in &self.unmanaged_x11_windows {
            self.space.unmap_elem(&entry.window);
            if entry.workspace == self.active_workspace
                && (!self.workspace_has_fullscreen(self.active_workspace) || entry.parent.is_some())
                && entry
                    .parent
                    .as_ref()
                    .is_none_or(|parent| self.space.element_location(parent).is_some())
            {
                let geometry = entry.window.x11_surface().unwrap().geometry();
                self.space
                    .map_element(entry.window.clone(), geometry.loc, false);
            }
        }
    }
}

/// Start XWayland without making it a prerequisite for native Wayland clients.
pub fn init(event_loop: &mut EventLoop<'static, Villain>, state: &mut Villain) {
    let environment = state.config.environment.clone();
    let Ok((xwayland, client)) = XWayland::spawn(
        &state.display_handle,
        None,
        environment,
        true,
        Stdio::null(),
        Stdio::null(),
        |_| (),
    ) else {
        state.xwayland_status = XWaylandStatus::Unavailable;
        tracing::warn!("XWayland is unavailable; continuing as a Wayland-only compositor");
        return;
    };

    let loop_handle = event_loop.handle();
    let xwm_handle = loop_handle.clone();
    if let Err(error) = loop_handle.insert_source(xwayland, move |event, _, state| match event {
        XWaylandEvent::Ready {
            x11_socket,
            display_number,
        } => match X11Wm::start_wm(xwm_handle.clone(), x11_socket, client.clone()) {
            Ok(xwm) => {
                state.xwm = Some(xwm);
                state.xwayland_display = Some(display_number);
                state.xwayland_status = XWaylandStatus::Available {
                    display: format!(":{display_number}"),
                };
                state
                    .config
                    .environment
                    .insert("DISPLAY".into(), format!(":{display_number}"));
                let screenshot_selection_is_active = current_data_device_selection_userdata(&state.seat)
                    .is_some_and(|payload| {
                        matches!(&*payload, crate::screenshot::SelectionPayload::Png(_))
                    });
                if screenshot_selection_is_active
                    && let Err(error) = crate::screenshot::announce_screenshot_clipboard_to_xwayland(state)
                {
                    tracing::warn!(%error, "could not announce existing screenshot clipboard to XWayland");
                }
                if state.owns_session {
                    crate::session::activate(state, false);
                }
                tracing::info!(display = %format_args!(":{display_number}"), "XWayland is ready");
            }
            Err(error) => {
                state.xwayland_status = XWaylandStatus::Unavailable;
                tracing::error!(%error, "could not start the XWayland window manager");
            }
        },
        XWaylandEvent::Error => {
            state.xwayland_status = XWaylandStatus::Unavailable;
            tracing::error!("XWayland exited before it became ready");
        }
    }) {
        state.xwayland_status = XWaylandStatus::Unavailable;
        tracing::error!(%error, "could not register XWayland with the event loop");
    }
}

impl XWaylandShellHandler for Villain {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }

    fn surface_associated(
        &mut self,
        _xwm: XwmId,
        _surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        window: X11Surface,
    ) {
        if let Some(window) = self.window_for_x11_surface(&window) {
            window.on_commit();
            // X11 focus can be selected before XWayland creates its wl_surface.
            // Repeat the enter now so Wayland keyboard events reach that client.
            let focus = crate::focus::KeyboardFocus::for_window(&window);
            if self.keyboard.current_focus() == focus {
                let keyboard = self.keyboard.clone();
                keyboard.set_focus(self, None, smithay::utils::SERIAL_COUNTER.next_serial());
                keyboard.set_focus(self, focus, smithay::utils::SERIAL_COUNTER.next_serial());
            }
            self.refresh_pointer_surface(0);
            self.request_repaint();
        }
    }
}

impl XwmHandler for Villain {
    fn xwm_state(&mut self, xwm: XwmId) -> &mut X11Wm {
        let state = self.xwm.as_mut().expect("XWM callback without an XWM");
        assert_eq!(state.id(), xwm);
        state
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Err(error) = window.set_mapped(true) {
            tracing::warn!(%error, window = window.window_id(), "could not map X11 window");
            return;
        }
        let fullscreen = window.is_fullscreen();
        // Initial EWMH maximization is replaced by the normal layout on map.
        self.add_x11_window(window.clone());
        if fullscreen && let Some(window) = self.window_for_x11_surface(&window) {
            self.set_window_fullscreen(&window, true);
        }
    }

    fn map_window_notify(&mut self, _xwm: XwmId, _window: X11Surface) {
        // Mapping can raise a background window after map_window_request.
        self.sync_x11_stacking();
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, surface: X11Surface) {
        let parent = surface
            .is_transient_for()
            .and_then(|id| self.window_for_x11_id(id))
            .or_else(|| {
                self.keyboard.current_focus().and_then(|focus| {
                    self.space
                        .elements()
                        .find(|window| {
                            crate::focus::KeyboardFocus::for_window(window).as_ref() == Some(&focus)
                        })
                        .cloned()
                })
            });
        let workspace = parent
            .as_ref()
            .and_then(|window| self.workspace_for_window(window))
            .unwrap_or(self.active_workspace);
        let window = Window::new_x11_window(surface);
        self.unmanaged_x11_windows.push(UnmanagedWindow {
            window,
            workspace,
            parent,
        });
        self.refresh_unmanaged_x11_windows();
        self.sync_x11_stacking();
        self.refresh_pointer(0);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.remove_x11_window(&window);
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.remove_x11_window(&window);
    }

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        width: Option<u32>,
        height: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        if !window.is_override_redirect()
            && let Some(managed) = self.window_for_x11_surface(&window)
        {
            if let Some(mut geometry) = self.floating_geometry(&managed) {
                geometry.loc.x = x.unwrap_or(geometry.loc.x);
                geometry.loc.y = y.unwrap_or(geometry.loc.y);
                geometry.size.w = width
                    .and_then(|value| i32::try_from(value).ok())
                    .unwrap_or(geometry.size.w);
                geometry.size.h = height
                    .and_then(|value| i32::try_from(value).ok())
                    .unwrap_or(geometry.size.h);
                self.set_floating_geometry(&managed, geometry);
            }
            // Also acknowledge requests denied by tiling/fullscreen policy.
            if let Err(error) = window.configure(None) {
                tracing::warn!(%error, "could not acknowledge X11 configure request");
            }
            return;
        }

        let mut geometry = window.geometry();
        geometry.loc.x = x.unwrap_or(geometry.loc.x);
        geometry.loc.y = y.unwrap_or(geometry.loc.y);
        geometry.size.w = width
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or(geometry.size.w);
        geometry.size.h = height
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or(geometry.size.h);
        if let Err(error) = window.configure(geometry) {
            tracing::warn!(%error, window = window.window_id(), "could not configure X11 window");
        }
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        surface: X11Surface,
        _geometry: Rectangle<i32, Logical>,
        _above: Option<X11Window>,
    ) {
        if !surface.is_override_redirect() {
            return;
        }
        if self.window_for_x11_surface(&surface).is_some() {
            self.refresh_unmanaged_x11_windows();
            if self
                .window_for_x11_surface(&surface)
                .is_some_and(|window| self.space.element_location(&window).is_none())
            {
                self.sync_x11_stacking();
            }
            self.refresh_pointer(0);
        }
    }

    fn property_notify(&mut self, _xwm: XwmId, surface: X11Surface, property: WmWindowProperty) {
        self.desktop_state_dirty = true;
        if matches!(
            property,
            WmWindowProperty::NormalHints
                | WmWindowProperty::TransientFor
                | WmWindowProperty::WindowType
        ) && let Some(window) = self.window_for_x11_surface(&surface)
        {
            self.refresh_window_hints(&window);
        }
    }

    fn maximize_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(window) = self.window_for_x11_surface(&surface) {
            self.set_window_maximized(&window, Self::window_opening_complete(&window));
        }
    }

    fn unmaximize_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(window) = self.window_for_x11_surface(&surface) {
            self.set_window_maximized(&window, false);
        }
    }

    fn fullscreen_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(window) = self.window_for_x11_surface(&surface) {
            self.set_window_fullscreen(&window, true);
        }
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(window) = self.window_for_x11_surface(&surface) {
            self.set_window_fullscreen(&window, false);
        }
    }

    fn minimize_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(window) = self.window_for_x11_surface(&surface) {
            self.apply_window_action(&window, WindowAction::Minimize);
        }
    }

    fn resize_request(&mut self, _xwm: XwmId, surface: X11Surface, button: u32, edge: ResizeEdge) {
        if self.x11_grab_button_matches(button)
            && let Some(window) = self.window_for_x11_surface(&surface)
        {
            self.start_window_grab(
                window,
                None,
                Some(crate::window_grab::ResizeEdges::from_x11(edge)),
            );
        }
    }

    fn move_request(&mut self, _xwm: XwmId, surface: X11Surface, button: u32) {
        if self.x11_grab_button_matches(button)
            && let Some(window) = self.window_for_x11_surface(&surface)
        {
            self.start_window_grab(window, None, None);
        }
    }

    fn allow_selection_access(&mut self, _xwm: XwmId, _selection: SelectionTarget) -> bool {
        self.keyboard
            .current_focus()
            .is_some_and(|focus| matches!(focus, crate::focus::KeyboardFocus::X11(_)))
    }

    fn send_selection(
        &mut self,
        _xwm: XwmId,
        selection: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
    ) {
        let server_selection = match selection {
            SelectionTarget::Clipboard => {
                current_data_device_selection_userdata(&self.seat).map(|data| data.clone())
            }
            SelectionTarget::Primary => {
                current_primary_selection_userdata(&self.seat).map(|data| data.clone())
            }
        };
        if let Some(payload) = server_selection {
            crate::screenshot::send_selection(self, &mime_type, fd, &payload);
            return;
        }

        let result = match selection {
            SelectionTarget::Clipboard => {
                request_data_device_client_selection(&self.seat, mime_type, fd)
                    .map_err(|error| error.to_string())
            }
            SelectionTarget::Primary => request_primary_client_selection(&self.seat, mime_type, fd)
                .map_err(|error| error.to_string()),
        };
        if let Err(error) = result {
            tracing::warn!(%error, ?selection, "could not transfer Wayland selection to XWayland");
        }
    }

    fn new_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        match selection {
            SelectionTarget::Clipboard => set_data_device_selection(
                &self.display_handle,
                &self.seat,
                mime_types,
                crate::screenshot::SelectionPayload::ForwardToXwayland,
            ),
            SelectionTarget::Primary => set_primary_selection(
                &self.display_handle,
                &self.seat,
                mime_types,
                crate::screenshot::SelectionPayload::ForwardToXwayland,
            ),
        }
    }

    fn cleared_selection(&mut self, _xwm: XwmId, selection: SelectionTarget) {
        match selection {
            SelectionTarget::Clipboard => {
                clear_data_device_selection(&self.display_handle, &self.seat)
            }
            SelectionTarget::Primary => clear_primary_selection(&self.display_handle, &self.seat),
        }
    }
}

smithay::delegate_xwayland_shell!(Villain);

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::reexports::{
        wayland_server::Display,
        x11rb::{connection::Connection, protocol::xproto::*, wrapper::ConnectionExt as _},
    };
    use std::time::{Duration, Instant};

    fn pump_until(
        event_loop: &mut EventLoop<'static, Villain>,
        state: &mut Villain,
        mut done: impl FnMut(&Villain) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(state) {
            assert!(Instant::now() < deadline, "XWayland test timed out");
            event_loop
                .dispatch(Duration::from_millis(5), state)
                .unwrap();
            state.space.refresh();
            if let Some(output) = state.space.outputs().next() {
                for window in state.space.elements() {
                    Villain::send_window_frame(window, output, state.start_time.elapsed());
                }
            }
            state.display_handle.flush_clients().unwrap();
        }
    }

    /// Uses a private, headless compositor and a real XWayland server. Run with
    /// an isolated XDG_RUNTIME_DIR; no desktop session or GPU is required.
    #[test]
    #[ignore = "requires Xwayland and an isolated XDG_RUNTIME_DIR"]
    fn xwayland_ready_announces_existing_screenshot_clipboard() {
        let mut event_loop = EventLoop::try_new().unwrap();
        let mut state = Villain::new(
            &mut event_loop,
            Display::new().unwrap(),
            crate::config::RuntimeConfig::load().unwrap(),
        );
        let output = smithay::output::Output::new(
            "test".into(),
            smithay::output::PhysicalProperties {
                size: (0, 0).into(),
                subpixel: smithay::output::Subpixel::Unknown,
                make: "test".into(),
                model: "test".into(),
            },
        );
        output.create_global::<Villain>(&state.display_handle);
        let mode = smithay::output::Mode {
            size: (800, 600).into(),
            refresh: 60_000,
        };
        output.change_current_state(Some(mode), None, None, Some((0, 0).into()));
        output.set_preferred(mode);
        state.space.map_output(&output, (0, 0));
        smithay::wayland::selection::data_device::set_data_device_selection::<Villain>(
            &state.display_handle,
            &state.seat,
            vec!["image/png".into()],
            crate::screenshot::SelectionPayload::Png(std::sync::Arc::from([1, 2, 3])),
        );

        init(&mut event_loop, &mut state);
        pump_until(&mut event_loop, &mut state, |state| {
            state.xwayland_display.is_some()
        });
        let display = format!(":{}", state.xwayland_display.unwrap());
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            sender
                .send(smithay::reexports::x11rb::connect(Some(&display)))
                .unwrap()
        });
        let mut connection = None;
        pump_until(&mut event_loop, &mut state, |_| {
            connection = receiver.try_recv().ok();
            connection.is_some()
        });
        let (conn, screen) = connection.unwrap().unwrap();
        let clipboard_atom = conn
            .intern_atom(false, b"CLIPBOARD")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        pump_until(&mut event_loop, &mut state, |_| {
            conn.get_selection_owner(clipboard_atom)
                .unwrap()
                .reply()
                .unwrap()
                .owner
                != smithay::reexports::x11rb::NONE
        });
        assert_ne!(
            conn.setup().roots[screen].root,
            smithay::reexports::x11rb::NONE
        );
    }

    /// Uses a private, headless compositor and a real XWayland server. Run with
    /// an isolated XDG_RUNTIME_DIR; no desktop session or GPU is required.
    #[test]
    #[ignore = "requires Xwayland and an isolated XDG_RUNTIME_DIR"]
    fn x11_workspace_input_isolation() {
        let mut event_loop = EventLoop::try_new().unwrap();
        let mut state = Villain::new(
            &mut event_loop,
            Display::new().unwrap(),
            crate::config::RuntimeConfig::load().unwrap(),
        );
        let output = smithay::output::Output::new(
            "test".into(),
            smithay::output::PhysicalProperties {
                size: (0, 0).into(),
                subpixel: smithay::output::Subpixel::Unknown,
                make: "test".into(),
                model: "test".into(),
            },
        );
        output.create_global::<Villain>(&state.display_handle);
        let mode = smithay::output::Mode {
            size: (800, 600).into(),
            refresh: 60_000,
        };
        output.change_current_state(Some(mode), None, None, Some((0, 0).into()));
        output.set_preferred(mode);
        state.space.map_output(&output, (0, 0));
        init(&mut event_loop, &mut state);
        pump_until(&mut event_loop, &mut state, |state| {
            state.xwayland_display.is_some()
        });
        let display = format!(":{}", state.xwayland_display.unwrap());
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            sender
                .send(smithay::reexports::x11rb::connect(Some(&display)))
                .unwrap()
        });
        let mut connection = None;
        pump_until(&mut event_loop, &mut state, |_| {
            connection = receiver.try_recv().ok();
            connection.is_some()
        });
        let (conn, screen) = connection.unwrap().unwrap();
        let root = conn.setup().roots[screen].root;
        let clipboard_atom = conn
            .intern_atom(false, b"CLIPBOARD")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        smithay::wayland::selection::data_device::set_data_device_selection::<Villain>(
            &state.display_handle,
            &state.seat,
            vec!["image/png".into()],
            crate::screenshot::SelectionPayload::Png(std::sync::Arc::from([1, 2, 3])),
        );
        crate::screenshot::announce_screenshot_clipboard_to_xwayland(&mut state)
            .expect("announce screenshot clipboard to XWayland");
        pump_until(&mut event_loop, &mut state, |_| {
            conn.get_selection_owner(clipboard_atom)
                .unwrap()
                .reply()
                .unwrap()
                .owner
                != smithay::reexports::x11rb::NONE
        });
        let create = |override_redirect: bool, parent: Option<u32>, maximized: bool| {
            let id = conn.generate_id().unwrap();
            conn.create_window(
                0,
                id,
                root,
                0,
                0,
                800,
                600,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &CreateWindowAux::new()
                    .override_redirect(u32::from(override_redirect))
                    .background_pixel(0x123456)
                    .event_mask(EventMask::BUTTON_PRESS | EventMask::KEY_PRESS),
            )
            .unwrap();
            if let Some(parent) = parent {
                conn.change_property32(
                    PropMode::REPLACE,
                    id,
                    AtomEnum::WM_TRANSIENT_FOR,
                    AtomEnum::WINDOW,
                    &[parent],
                )
                .unwrap();
            }
            if maximized {
                let atom =
                    |name: &[u8]| conn.intern_atom(false, name).unwrap().reply().unwrap().atom;
                conn.change_property32(
                    PropMode::REPLACE,
                    id,
                    atom(b"_NET_WM_STATE"),
                    AtomEnum::ATOM,
                    &[
                        atom(b"_NET_WM_STATE_MAXIMIZED_HORZ"),
                        atom(b"_NET_WM_STATE_MAXIMIZED_VERT"),
                    ],
                )
                .unwrap();
            }
            conn.map_window(id).unwrap();
            conn.flush().unwrap();
            id
        };
        let first = create(false, None, true);
        pump_until(&mut event_loop, &mut state, |state| {
            state
                .window_for_x11_id(first)
                .is_some_and(|window| window.x11_surface().unwrap().wl_surface().is_some())
        });
        let image_png_atom = conn
            .intern_atom(false, b"image/png")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        let property_atom = conn
            .intern_atom(false, b"KNAVE_TEST_IMAGE")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        conn.convert_selection(
            first,
            clipboard_atom,
            image_png_atom,
            property_atom,
            smithay::reexports::x11rb::CURRENT_TIME,
        )
        .unwrap();
        conn.flush().unwrap();
        let mut selection_notify = None;
        pump_until(&mut event_loop, &mut state, |_| {
            while let Some(event) = conn.poll_for_event().unwrap() {
                if let smithay::reexports::x11rb::protocol::Event::SelectionNotify(event) = event {
                    selection_notify = Some(event);
                }
            }
            selection_notify.is_some()
                && conn
                    .get_property(false, first, property_atom, AtomEnum::ANY, 0, 1024)
                    .unwrap()
                    .reply()
                    .unwrap()
                    .value
                    == [1, 2, 3]
        });
        assert_eq!(selection_notify.unwrap().property, property_atom);
        assert_eq!(
            conn.get_property(false, first, property_atom, AtomEnum::ANY, 0, 1024)
                .unwrap()
                .reply()
                .unwrap()
                .value,
            [1, 2, 3],
            "XWayland should receive the screenshot PNG bytes"
        );
        let first_window = state.window_for_x11_id(first).unwrap();
        assert!(!first_window.x11_surface().unwrap().is_maximized());
        assert!(
            !state
                .window_info()
                .iter()
                .find(|w| w.id == state.window_id(&first_window).unwrap())
                .unwrap()
                .maximized
        );
        state.switch_workspace(4);
        let second = create(false, None, false);
        pump_until(&mut event_loop, &mut state, |state| {
            state
                .window_for_x11_id(second)
                .is_some_and(|window| window.x11_surface().unwrap().wl_surface().is_some())
        });
        let popup = create(true, Some(second), false);
        pump_until(&mut event_loop, &mut state, |state| {
            state
                .window_for_x11_id(popup)
                .is_some_and(|window| window.x11_surface().unwrap().wl_surface().is_some())
        });
        state.pointer_location = (100.0, 100.0).into();

        for (workspace, expected, hidden) in
            [(0, first, second), (4, second, first), (0, first, second)]
        {
            state.switch_workspace(workspace);
            state.display_handle.flush_clients().unwrap();
            assert_eq!(
                conn.get_input_focus().unwrap().reply().unwrap().focus,
                expected
            );
            let pointer_target = if workspace == 4 { popup } else { expected };
            let window = state.window_for_x11_id(pointer_target).unwrap();
            assert_eq!(
                state.pointer.current_focus(),
                window.x11_surface().unwrap().wl_surface()
            );
            assert!(
                state
                    .space
                    .element_location(&state.window_for_x11_id(hidden).unwrap())
                    .is_none()
            );
            let popup_window = state.window_for_x11_id(popup).unwrap();
            assert_eq!(
                state.space.element_location(&popup_window).is_some(),
                workspace == 4
            );
            let tree = conn.query_tree(root).unwrap().reply().unwrap().children;
            let frame = |id| {
                state
                    .window_for_x11_id(id)
                    .unwrap()
                    .x11_surface()
                    .unwrap()
                    .mapped_window_id()
                    .unwrap()
            };
            assert!(
                tree.iter().position(|id| *id == frame(expected))
                    > tree.iter().position(|id| *id == frame(hidden))
            );
            while conn.poll_for_event().unwrap().is_some() {}
            let pointer = state.pointer.clone();
            for button_state in [
                smithay::backend::input::ButtonState::Pressed,
                smithay::backend::input::ButtonState::Released,
            ] {
                pointer.button(
                    &mut state,
                    &smithay::input::pointer::ButtonEvent {
                        serial: smithay::utils::SERIAL_COUNTER.next_serial(),
                        time: 1,
                        button: 0x110,
                        state: button_state,
                    },
                );
            }
            pointer.frame(&mut state);
            state.display_handle.flush_clients().unwrap();
            let mut clicked = None;
            pump_until(&mut event_loop, &mut state, |_| {
                while let Some(event) = conn.poll_for_event().unwrap() {
                    if let smithay::reexports::x11rb::protocol::Event::ButtonPress(event) = event {
                        clicked = Some(event.event);
                    }
                }
                clicked.is_some()
            });
            assert_eq!(
                clicked,
                Some(pointer_target),
                "click reached the wrong X11 window"
            );
            let keyboard = state.keyboard.clone();
            for key_state in [
                smithay::backend::input::KeyState::Pressed,
                smithay::backend::input::KeyState::Released,
            ] {
                keyboard.input::<(), _>(
                    &mut state,
                    38u32.into(),
                    key_state,
                    smithay::utils::SERIAL_COUNTER.next_serial(),
                    2,
                    |_, _, _| smithay::input::keyboard::FilterResult::Forward,
                );
            }
            state.display_handle.flush_clients().unwrap();
            let mut typed = None;
            pump_until(&mut event_loop, &mut state, |_| {
                while let Some(event) = conn.poll_for_event().unwrap() {
                    if let smithay::reexports::x11rb::protocol::Event::KeyPress(event) = event {
                        typed = Some(event.event);
                    }
                }
                typed.is_some()
            });
            assert_eq!(typed, Some(expected), "key reached the wrong X11 window");
        }
        // A configure event from a hidden popup must not remap it or steal focus.
        conn.configure_window(popup, &ConfigureWindowAux::new().x(20))
            .unwrap();
        conn.flush().unwrap();
        pump_until(&mut event_loop, &mut state, |state| {
            state
                .window_for_x11_id(popup)
                .unwrap()
                .x11_surface()
                .unwrap()
                .geometry()
                .loc
                .x
                == 20
        });
        assert!(
            state
                .space
                .element_location(&state.window_for_x11_id(popup).unwrap())
                .is_none()
        );
        assert_eq!(
            conn.get_input_focus().unwrap().reply().unwrap().focus,
            first
        );
        // A background app can create a popup after we switch away.
        let late_popup = create(true, Some(second), false);
        pump_until(&mut event_loop, &mut state, |state| {
            state.window_for_x11_id(late_popup).is_some()
        });
        assert!(
            state
                .space
                .element_location(&state.window_for_x11_id(late_popup).unwrap())
                .is_none()
        );
        assert_eq!(
            conn.get_input_focus().unwrap().reply().unwrap().focus,
            first
        );
        state.pressed_buttons.insert(0x110);
        state.pointer.clone().button(
            &mut state,
            &smithay::input::pointer::ButtonEvent {
                serial: smithay::utils::SERIAL_COUNTER.next_serial(),
                time: 3,
                button: 0x110,
                state: smithay::backend::input::ButtonState::Pressed,
            },
        );
        assert!(state.pointer.is_grabbed());
        assert!(state.minimize_focused_window());
        assert!(!state.pointer.is_grabbed());
        assert!(state.pressed_buttons.is_empty());
        assert!(state.keyboard.current_focus().is_none());
        assert!(state.pointer.current_focus().is_none());
        assert!(state.restore_last_minimized_window());
        assert_eq!(
            conn.get_input_focus().unwrap().reply().unwrap().focus,
            first
        );
        // Exercise real EWMH fullscreen requests and ICCCM floating hints.
        let other = create(false, None, false);
        pump_until(&mut event_loop, &mut state, |state| {
            state.window_for_x11_id(other).is_some()
        });
        state
            .dispatch(crate::dispatch::Dispatch::ResizeMaster(10))
            .unwrap();
        pump_until(&mut event_loop, &mut state, |_| {
            conn.get_geometry(first).unwrap().reply().unwrap().width == 480
                && conn.get_geometry(other).unwrap().reply().unwrap().width == 320
        });
        assert_eq!(
            conn.get_geometry(first).unwrap().reply().unwrap().width,
            480
        );
        assert_eq!(
            conn.get_geometry(other).unwrap().reply().unwrap().width,
            320
        );
        state
            .dispatch(crate::dispatch::Dispatch::ResetMaster)
            .unwrap();
        let dialog = create(false, Some(first), false);
        pump_until(&mut event_loop, &mut state, |state| {
            state
                .window_for_x11_id(dialog)
                .is_some_and(|window| window.x11_surface().unwrap().wl_surface().is_some())
        });
        conn.configure_window(
            dialog,
            &ConfigureWindowAux::new()
                .x(100)
                .y(100)
                .width(300)
                .height(200),
        )
        .unwrap();
        conn.flush().unwrap();
        pump_until(&mut event_loop, &mut state, |state| {
            state
                .window_for_x11_id(dialog)
                .unwrap()
                .x11_surface()
                .unwrap()
                .geometry()
                .size
                == (300, 200).into()
        });
        let dialog_window = state.window_for_x11_id(dialog).unwrap();
        let saved = state.floating_geometry(&dialog_window).unwrap();
        let fullscreen_atom = conn
            .intern_atom(false, b"_NET_WM_STATE_FULLSCREEN")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        let wm_state = conn
            .intern_atom(false, b"_NET_WM_STATE")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        let fullscreen = |id, enabled| {
            conn.send_event(
                false,
                root,
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                ClientMessageEvent::new(
                    32,
                    id,
                    wm_state,
                    [u32::from(enabled), fullscreen_atom, 0, 1, 0],
                ),
            )
            .unwrap();
            conn.flush().unwrap();
        };
        let max_h = conn
            .intern_atom(false, b"_NET_WM_STATE_MAXIMIZED_HORZ")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        let max_v = conn
            .intern_atom(false, b"_NET_WM_STATE_MAXIMIZED_VERT")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        let maximize = |id, enabled| {
            std::thread::sleep(Duration::from_secs(1));
            conn.send_event(
                false,
                root,
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                ClientMessageEvent::new(32, id, wm_state, [u32::from(enabled), max_h, max_v, 1, 0]),
            )
            .unwrap();
            conn.flush().unwrap();
        };
        maximize(first, true);
        pump_until(&mut event_loop, &mut state, |state| {
            state
                .window_for_x11_id(first)
                .unwrap()
                .x11_surface()
                .unwrap()
                .is_maximized()
        });
        assert_eq!(
            conn.get_geometry(first).unwrap().reply().unwrap().width,
            800
        );
        let first_window = state.window_for_x11_id(first).unwrap();
        let new_child = create(false, Some(first), false);
        pump_until(&mut event_loop, &mut state, |state| {
            state
                .window_for_x11_id(new_child)
                .is_some_and(|window| window.x11_surface().unwrap().wl_surface().is_some())
        });
        assert!(first_window.x11_surface().unwrap().is_maximized());
        assert!(
            state
                .space
                .element_location(&state.window_for_x11_id(new_child).unwrap())
                .is_some()
        );
        assert_eq!(
            conn.get_input_focus().unwrap().reply().unwrap().focus,
            new_child
        );
        conn.destroy_window(new_child).unwrap();
        conn.flush().unwrap();
        pump_until(&mut event_loop, &mut state, |state| {
            state.window_for_x11_id(new_child).is_none()
        });
        let new_app = create(false, None, false);
        pump_until(&mut event_loop, &mut state, |state| {
            state
                .window_for_x11_id(new_app)
                .is_some_and(|window| window.x11_surface().unwrap().wl_surface().is_some())
        });
        assert!(!first_window.x11_surface().unwrap().is_maximized());
        assert!(
            state
                .space
                .element_location(&state.window_for_x11_id(new_app).unwrap())
                .is_some()
        );
        assert_eq!(
            conn.get_input_focus().unwrap().reply().unwrap().focus,
            new_app
        );
        conn.destroy_window(new_app).unwrap();
        conn.flush().unwrap();
        pump_until(&mut event_loop, &mut state, |state| {
            state.window_for_x11_id(new_app).is_none()
        });
        maximize(first, true);
        pump_until(&mut event_loop, &mut state, |_| {
            first_window.x11_surface().unwrap().is_maximized()
        });

        assert!(!first_window.x11_surface().unwrap().is_fullscreen());
        assert!(state.space.element_location(&dialog_window).is_some());
        fullscreen(first, true);
        pump_until(&mut event_loop, &mut state, |_| {
            first_window.x11_surface().unwrap().is_fullscreen()
        });
        fullscreen(first, false);
        pump_until(&mut event_loop, &mut state, |_| {
            !first_window.x11_surface().unwrap().is_fullscreen()
        });
        assert!(first_window.x11_surface().unwrap().is_maximized());
        assert_eq!(
            conn.get_geometry(first).unwrap().reply().unwrap().width,
            800
        );
        maximize(first, false);
        pump_until(&mut event_loop, &mut state, |_| {
            !first_window.x11_surface().unwrap().is_maximized()
        });
        assert_eq!(
            conn.get_geometry(first).unwrap().reply().unwrap().width,
            400
        );
        maximize(dialog, true);
        pump_until(&mut event_loop, &mut state, |_| {
            dialog_window.x11_surface().unwrap().is_maximized()
        });
        assert!(state.floating_geometry(&dialog_window).is_none());
        maximize(dialog, false);
        pump_until(&mut event_loop, &mut state, |_| {
            !dialog_window.x11_surface().unwrap().is_maximized()
        });
        assert_eq!(state.floating_geometry(&dialog_window), Some(saved));
        fullscreen(first, true);
        pump_until(&mut event_loop, &mut state, |state| {
            state
                .window_for_x11_id(first)
                .unwrap()
                .x11_surface()
                .unwrap()
                .is_fullscreen()
        });
        assert_eq!(
            conn.get_geometry(first).unwrap().reply().unwrap().width,
            800
        );
        assert!(state.space.element_location(&dialog_window).is_some());
        assert!(
            state
                .space
                .element_location(&state.window_for_x11_id(other).unwrap())
                .is_none()
        );
        fullscreen(first, false);
        pump_until(&mut event_loop, &mut state, |state| {
            !state
                .window_for_x11_id(first)
                .unwrap()
                .x11_surface()
                .unwrap()
                .is_fullscreen()
        });
        assert_eq!(
            conn.get_geometry(first).unwrap().reply().unwrap().width,
            400
        );
        fullscreen(dialog, true);
        pump_until(&mut event_loop, &mut state, |state| {
            state
                .window_for_x11_id(dialog)
                .unwrap()
                .x11_surface()
                .unwrap()
                .is_fullscreen()
        });
        assert_eq!(
            conn.get_geometry(dialog).unwrap().reply().unwrap().width,
            800
        );
        fullscreen(dialog, false);
        pump_until(&mut event_loop, &mut state, |state| {
            !state
                .window_for_x11_id(dialog)
                .unwrap()
                .x11_surface()
                .unwrap()
                .is_fullscreen()
        });
        assert_eq!(state.floating_geometry(&dialog_window), Some(saved));
        assert_eq!(
            conn.get_geometry(dialog).unwrap().reply().unwrap().width,
            300
        );

        let fixed = create(false, None, false);
        let hints = smithay::reexports::x11rb::properties::WmSizeHints {
            min_size: Some((240, 180)),
            max_size: Some((240, 180)),
            ..Default::default()
        };
        hints.set_normal_hints(&conn, fixed).unwrap();
        conn.flush().unwrap();
        pump_until(&mut event_loop, &mut state, |state| {
            state
                .window_for_x11_id(fixed)
                .is_some_and(|window| state.floating_geometry(&window).is_some())
        });
        assert_eq!(
            conn.get_geometry(fixed).unwrap().reply().unwrap().width,
            240
        );
        // Focus must agree in Smithay, the X server and the preview scene.
        let fixed_window = state.window_for_x11_id(fixed).unwrap();
        for (raised, below) in [
            (&dialog_window, &fixed_window),
            (&fixed_window, &dialog_window),
        ] {
            state.focus_managed_window(raised);
            let tree = conn.query_tree(root).unwrap().reply().unwrap().children;
            let frame = |window: &smithay::desktop::Window| {
                window.x11_surface().unwrap().mapped_window_id().unwrap()
            };
            assert!(
                tree.iter().position(|id| *id == frame(raised))
                    > tree.iter().position(|id| *id == frame(below))
            );
            assert_eq!(
                conn.get_input_focus().unwrap().reply().unwrap().focus,
                raised.x11_surface().unwrap().window_id()
            );
            state.relayout_active_workspace();
            let order: Vec<_> = state.space.elements().cloned().collect();
            assert!(order.iter().position(|w| w == raised) > order.iter().position(|w| w == below));
        }
        // A client cannot move another window or start a drag without a button.
        state.start_window_grab(dialog_window.clone(), None, None);
        assert!(!state.pointer.is_grabbed());
        state.pointer_location = (110.0, 110.0).into();
        state.refresh_pointer(10);
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        state.pressed_buttons.insert(0x110);
        state.pointer.clone().button(
            &mut state,
            &smithay::input::pointer::ButtonEvent {
                serial,
                time: 10,
                button: 0x110,
                state: smithay::backend::input::ButtonState::Pressed,
            },
        );
        let fixed_before = state.floating_geometry(&fixed_window);
        state.start_window_grab(fixed_window.clone(), Some(serial), None);
        state.start_window_grab(dialog_window.clone(), Some(serial), None);
        state.pointer_location = (160.0, 140.0).into();
        state.refresh_pointer(11);
        assert_eq!(
            state.floating_geometry(&dialog_window).unwrap().loc,
            saved.loc + smithay::utils::Point::from((50, 30))
        );
        assert_eq!(state.floating_geometry(&fixed_window), fixed_before);
        state.release_pointer_buttons();
        assert!(!state.pointer.is_grabbed());
        let moved = state.floating_geometry(&dialog_window).unwrap();
        state.pointer_location = (moved.loc.x as f64 + 10.0, moved.loc.y as f64 + 10.0).into();
        state.refresh_pointer(12);
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        state.pressed_buttons.insert(0x110);
        state.pointer.clone().button(
            &mut state,
            &smithay::input::pointer::ButtonEvent {
                serial,
                time: 12,
                button: 0x110,
                state: smithay::backend::input::ButtonState::Pressed,
            },
        );
        state.start_window_grab(
            dialog_window.clone(),
            Some(serial),
            Some(crate::window_grab::ResizeEdges::from_x11(
                ResizeEdge::BottomRight,
            )),
        );
        state.pointer_location += (50.0, 40.0).into();
        state.refresh_pointer(13);
        assert_eq!(
            state.floating_geometry(&dialog_window).unwrap().size,
            (350, 240).into()
        );
        state.switch_workspace(4);
        assert!(!state.pointer.is_grabbed());
        assert!(state.space.element_location(&dialog_window).is_none());
    }
}
