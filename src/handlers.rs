//! The smallest set of protocol handlers needed to show client windows.

use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    desktop::{PopupKind, find_popup_root_surface, layer_map_for_output},
    reexports::calloop::Interest,
    reexports::wayland_server::{
        Client, Resource,
        protocol::{wl_buffer, wl_surface::WlSurface},
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{
            BufferAssignment, CompositorClientState, CompositorHandler, CompositorState,
            SurfaceAttributes, add_blocker, add_pre_commit_hook, get_role, with_states,
        },
        dmabuf::get_dmabuf,
        fractional_scale::{FractionalScaleHandler, with_fractional_scale},
        output::OutputHandler,
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XDG_POPUP_ROLE, XdgPopupSurfaceData,
            XdgShellHandler, XdgShellState, XdgToplevelSurfaceData,
        },
        shm::{ShmHandler, ShmState},
    },
};
use smithay::{
    input::Seat,
    reexports::wayland_server::protocol::wl_seat,
    utils::Serial,
    wayland::selection::{
        SelectionHandler,
        data_device::{
            ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
        },
        ext_data_control,
        primary_selection::{PrimarySelectionHandler, PrimarySelectionState},
        wlr_data_control,
    },
    xwayland::XWaylandClientData,
};
use smithay::{
    reexports::{
        wayland_protocols_misc::zwp_virtual_keyboard_v1::server::{
            zwp_virtual_keyboard_manager_v1::{self, ZwpVirtualKeyboardManagerV1},
            zwp_virtual_keyboard_v1::{self, ZwpVirtualKeyboardV1},
        },
        wayland_server::{DataInit, Dispatch, DisplayHandle, backend::ClientId},
    },
    wayland::virtual_keyboard::{
        VirtualKeyboardManagerGlobalData, VirtualKeyboardManagerState, VirtualKeyboardUserData,
    },
};
use std::os::fd::OwnedFd;
use std::sync::atomic::Ordering;

type PopupRect = smithay::utils::Rectangle<i32, smithay::utils::Logical>;

fn safe_popup_geometry(positioner: PositionerState, target: Option<[i64; 4]>) -> Option<PopupRect> {
    use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_positioner::{
        Anchor, ConstraintAdjustment, Gravity,
    };

    let has_anchor_left = matches!(
        positioner.anchor_edges,
        Anchor::Left | Anchor::TopLeft | Anchor::BottomLeft
    );
    let has_anchor_right = matches!(
        positioner.anchor_edges,
        Anchor::Right | Anchor::TopRight | Anchor::BottomRight
    );
    let has_anchor_top = matches!(
        positioner.anchor_edges,
        Anchor::Top | Anchor::TopLeft | Anchor::TopRight
    );
    let has_anchor_bottom = matches!(
        positioner.anchor_edges,
        Anchor::Bottom | Anchor::BottomLeft | Anchor::BottomRight
    );
    let has_gravity_left = matches!(
        positioner.gravity,
        Gravity::Left | Gravity::TopLeft | Gravity::BottomLeft
    );
    let has_gravity_right = matches!(
        positioner.gravity,
        Gravity::Right | Gravity::TopRight | Gravity::BottomRight
    );
    let has_gravity_top = matches!(
        positioner.gravity,
        Gravity::Top | Gravity::TopLeft | Gravity::TopRight
    );
    let has_gravity_bottom = matches!(
        positioner.gravity,
        Gravity::Bottom | Gravity::BottomLeft | Gravity::BottomRight
    );

    let anchor = positioner.anchor_rect;
    let size = positioner.rect_size;
    let anchor_x = i64::from(anchor.loc.x)
        + if has_anchor_left {
            0
        } else if has_anchor_right {
            i64::from(anchor.size.w)
        } else {
            i64::from(anchor.size.w) / 2
        };
    let anchor_y = i64::from(anchor.loc.y)
        + if has_anchor_top {
            0
        } else if has_anchor_bottom {
            i64::from(anchor.size.h)
        } else {
            i64::from(anchor.size.h) / 2
        };
    let mut x = anchor_x + i64::from(positioner.offset.x);
    let mut y = anchor_y + i64::from(positioner.offset.y);
    if has_gravity_left {
        x -= i64::from(size.w);
    } else if !has_gravity_right {
        x -= i64::from(size.w) / 2;
    }
    if has_gravity_top {
        y -= i64::from(size.h);
    } else if !has_gravity_bottom {
        y -= i64::from(size.h) / 2;
    }
    let width = i64::from(size.w);
    let height = i64::from(size.h);
    let Some([target_x, target_y, target_width, target_height]) = target else {
        return rect_from_i64(x, y, width, height);
    };
    let offsets = |x: i64, y: i64, width: i64, height: i64| {
        (
            target_x - x,
            x + width - (target_x + target_width),
            target_y - y,
            y + height - (target_y + target_height),
        )
    };
    let (mut left, mut right, mut top, mut bottom) = offsets(x, y, width, height);

    if (left > 0 || right > 0)
        && positioner
            .constraint_adjustment
            .contains(ConstraintAdjustment::FlipX)
    {
        let mut flipped = positioner;
        flipped.anchor_edges = match flipped.anchor_edges {
            Anchor::Left => Anchor::Right,
            Anchor::Right => Anchor::Left,
            Anchor::TopLeft => Anchor::TopRight,
            Anchor::TopRight => Anchor::TopLeft,
            Anchor::BottomLeft => Anchor::BottomRight,
            Anchor::BottomRight => Anchor::BottomLeft,
            other => other,
        };
        flipped.gravity = match flipped.gravity {
            Gravity::Left => Gravity::Right,
            Gravity::Right => Gravity::Left,
            Gravity::TopLeft => Gravity::TopRight,
            Gravity::TopRight => Gravity::TopLeft,
            Gravity::BottomLeft => Gravity::BottomRight,
            Gravity::BottomRight => Gravity::BottomLeft,
            other => other,
        };
        if let Some(rect) = safe_popup_geometry_unconstrained(flipped) {
            let (new_left, new_right, _, _) = offsets(
                i64::from(rect.loc.x),
                i64::from(rect.loc.y),
                i64::from(rect.size.w),
                i64::from(rect.size.h),
            );
            if new_left <= 0 && new_right <= 0 {
                x = i64::from(rect.loc.x);
                y = i64::from(rect.loc.y);
                left = 0;
                right = 0;
            }
        }
    }
    if (top > 0 || bottom > 0)
        && positioner
            .constraint_adjustment
            .contains(ConstraintAdjustment::FlipY)
    {
        let mut flipped = positioner;
        flipped.anchor_edges = match flipped.anchor_edges {
            Anchor::Top => Anchor::Bottom,
            Anchor::Bottom => Anchor::Top,
            Anchor::TopLeft => Anchor::BottomLeft,
            Anchor::TopRight => Anchor::BottomRight,
            Anchor::BottomLeft => Anchor::TopLeft,
            Anchor::BottomRight => Anchor::TopRight,
            other => other,
        };
        flipped.gravity = match flipped.gravity {
            Gravity::Top => Gravity::Bottom,
            Gravity::Bottom => Gravity::Top,
            Gravity::TopLeft => Gravity::BottomLeft,
            Gravity::TopRight => Gravity::BottomRight,
            Gravity::BottomLeft => Gravity::TopLeft,
            Gravity::BottomRight => Gravity::TopRight,
            other => other,
        };
        if let Some(rect) = safe_popup_geometry_unconstrained(flipped) {
            let (_, _, new_top, new_bottom) = offsets(
                i64::from(rect.loc.x),
                i64::from(rect.loc.y),
                i64::from(rect.size.w),
                i64::from(rect.size.h),
            );
            if new_top <= 0 && new_bottom <= 0 {
                x = i64::from(rect.loc.x);
                y = i64::from(rect.loc.y);
                top = 0;
                bottom = 0;
            }
        }
    }
    if positioner
        .constraint_adjustment
        .contains(ConstraintAdjustment::SlideX)
    {
        if left > 0 {
            x += left;
        } else if right > 0 {
            x -= right.min(-left);
        }
        (left, right, _, _) = offsets(x, y, width, height);
    }
    if positioner
        .constraint_adjustment
        .contains(ConstraintAdjustment::SlideY)
    {
        if top > 0 {
            y += top;
        } else if bottom > 0 {
            y -= bottom.min(-top);
        }
        (_, _, top, bottom) = offsets(x, y, width, height);
    }
    let mut width = width;
    let mut height = height;
    if positioner
        .constraint_adjustment
        .contains(ConstraintAdjustment::ResizeX)
    {
        if left > 0 && left < width {
            x += left;
            width -= left;
        }
        if right > 0 && right < width {
            width -= right;
        }
    }
    if positioner
        .constraint_adjustment
        .contains(ConstraintAdjustment::ResizeY)
    {
        if top > 0 && top < height {
            y += top;
            height -= top;
        }
        if bottom > 0 && bottom < height {
            height -= bottom;
        }
    }
    rect_from_i64(x, y, width, height)
}

fn safe_popup_geometry_unconstrained(positioner: PositionerState) -> Option<PopupRect> {
    safe_popup_geometry(positioner, None)
}

fn rect_from_i64(x: i64, y: i64, width: i64, height: i64) -> Option<PopupRect> {
    Some(smithay::utils::Rectangle::new(
        (i32::try_from(x).ok()?, i32::try_from(y).ok()?).into(),
        (i32::try_from(width).ok()?, i32::try_from(height).ok()?).into(),
    ))
}

use crate::state::{ClientState, Villain};
use crate::workspaces::WindowAction;

impl CompositorHandler for Villain {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        if let Some(state) = client.get_data::<ClientState>() {
            &state.compositor_state
        } else if let Some(state) = client.get_data::<XWaylandClientData>() {
            &state.compositor_state
        } else {
            unreachable!("Wayland client has no compositor state")
        }
    }

    fn new_surface(&mut self, surface: &WlSurface) {
        // Defer a commit until its DMA-BUF's implicit fences signal, so the
        // renderer never samples (and waits on) a buffer the client's GPU work
        // has not finished. The previous state stays current meanwhile and the
        // event loop keeps serving input and other clients.
        add_pre_commit_hook::<Self, _>(surface, |state, _dh, surface| {
            let dmabuf = with_states(surface, |states| {
                match states
                    .cached_state
                    .get::<SurfaceAttributes>()
                    .pending()
                    .buffer
                    .as_ref()
                {
                    Some(BufferAssignment::NewBuffer(buffer)) => get_dmabuf(buffer).cloned().ok(),
                    _ => None,
                }
            });
            let (Some(dmabuf), Some(client)) = (dmabuf, surface.client()) else {
                return;
            };
            let Ok((blocker, source)) = dmabuf.generate_blocker(Interest::READ) else {
                return;
            };
            let inserted = state.loop_handle.insert_source(source, move |_, _, state| {
                let dh = state.display_handle.clone();
                state
                    .client_compositor_state(&client)
                    .blocker_cleared(state, &dh);
                Ok(())
            });
            if inserted.is_ok() {
                add_blocker(surface, blocker);
            }
        });
    }

    fn commit(&mut self, surface: &WlSurface) {
        // This turns a client's wl_buffer commit into the state Smithay's
        // renderer can later inspect.
        on_commit_buffer_handler::<Self>(surface);
        self.popups.commit(surface);
        if let Some(smithay::desktop::PopupKind::Xdg(popup)) = self.popups.find_popup(surface) {
            if !popup.is_initial_configure_sent() {
                let _ = popup.send_configure();
            }
            self.request_repaint();
        }
        if self.layer_commit(surface) {
            return;
        }

        let root = std::iter::successors(Some(surface.clone()), |surface| {
            smithay::wayland::compositor::get_parent(surface)
        })
        .last()
        .unwrap();
        let visible_window = self
            .window_for_surface(&root)
            .is_some_and(|window| self.space.element_location(&window).is_some());
        let overview_window = !visible_window
            && self.window_for_surface(&root).is_some_and(|window| {
                self.workspace_for_window(&window).is_some_and(|index| {
                    self.overview_panes.iter().any(|pane| {
                        pane.workspace.0 as usize == index + 1
                            && self.workspace_preview_scene(index).is_some_and(|scene| {
                                scene.windows.iter().any(|(visible, _)| visible == &window)
                            })
                    })
                })
            });
        if visible_window
            || overview_window
            || self
                .window_for_surface(&root)
                .and_then(|window| self.workspace_for_window(&window))
                .is_some_and(|index| self.workspace_scene_visible(index))
            || self.layer_surface_visible(&root)
            || self.cursor.uses_surface(&root)
        {
            self.request_repaint();
        }

        // The first commit is the handshake: we tell the client which state
        // the compositor accepts, then the client can commit its first buffer.
        if let Some(window) = self.window_for_surface(surface) {
            // Update the window's bounding box from the newly committed
            // surface. Space uses this geometry to decide what gets rendered
            // on each output.
            window.on_commit();
            self.refresh_window_hints(&window);
            self.finish_new_window_focus(&window);

            if let Some(toplevel) = window.toplevel() {
                let initial_configure_sent = with_states(surface, |states| {
                    states
                        .data_map
                        .get::<XdgToplevelSurfaceData>()
                        .expect("XDG toplevel data")
                        .lock()
                        .unwrap()
                        .initial_configure_sent
                });

                if !initial_configure_sent {
                    toplevel.send_configure();
                }
            }
            // A repaint can change the pointer's surface-local coordinates,
            // but it must not override focus chosen by a dispatcher.
            self.refresh_pointer_surface(0);
        }
    }

    fn destroyed(&mut self, surface: &WlSurface) {
        self.fractional_scale_surfaces
            .retain(|weak| weak.id() != surface.id());
    }
}

impl FractionalScaleHandler for Villain {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        let scale = self.config.output_scale;
        with_states(&surface, |states| {
            with_fractional_scale(states, |fractional| fractional.set_preferred_scale(scale));
        });
        self.fractional_scale_surfaces.push(surface.downgrade());
    }
}

impl BufferHandler for Villain {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for Villain {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl SelectionHandler for Villain {
    type SelectionUserData = crate::screenshot::SelectionPayload;

    fn new_selection(
        &mut self,
        selection: smithay::wayland::selection::SelectionTarget,
        source: Option<smithay::wayland::selection::SelectionSource>,
        _seat: Seat<Self>,
    ) {
        let Some(xwm) = self.xwm.as_mut() else {
            return;
        };
        let mime_types = source.map(|source| source.mime_types());
        if let Err(error) = xwm.new_selection(selection, mime_types) {
            tracing::warn!(%error, ?selection, "could not export Wayland selection to XWayland");
        }
    }

    fn send_selection(
        &mut self,
        selection: smithay::wayland::selection::SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
        _seat: Seat<Self>,
        user_data: &Self::SelectionUserData,
    ) {
        match user_data {
            crate::screenshot::SelectionPayload::Png(_) => {
                crate::screenshot::send_selection(self, &mime_type, fd, user_data);
            }
            crate::screenshot::SelectionPayload::ForwardToXwayland => {
                let Some(xwm) = self.xwm.as_mut() else {
                    return;
                };
                if let Err(error) =
                    xwm.send_selection(selection, mime_type, fd, self.loop_handle.clone())
                {
                    tracing::warn!(%error, ?selection, "could not transfer XWayland selection");
                }
            }
        }
    }
}

impl DataDeviceHandler for Villain {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl PrimarySelectionHandler for Villain {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.primary_selection_state
    }
}

impl wlr_data_control::DataControlHandler for Villain {
    fn data_control_state(&self) -> &wlr_data_control::DataControlState {
        &self.wlr_data_control_state
    }
}

impl ext_data_control::DataControlHandler for Villain {
    fn data_control_state(&self) -> &ext_data_control::DataControlState {
        &self.ext_data_control_state
    }
}

impl ClientDndGrabHandler for Villain {}

impl ServerDndGrabHandler for Villain {
    fn send(&mut self, _mime_type: String, _fd: OwnedFd, _seat: Seat<Self>) {}
}

impl smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitHandler for Villain {
    fn keyboard_shortcuts_inhibit_state(
        &mut self,
    ) -> &mut smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitState {
        &mut self.keyboard_shortcuts_inhibit_state
    }

    fn new_inhibitor(
        &mut self,
        inhibitor: smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitor,
    ) {
        // Recorders, terminal emulators and remote clients request the
        // inhibitor while they are focused: it takes effect immediately,
        // without a per-client prompt. Activation is durable: focus loss and
        // destruction only make inhibition irrelevant without notifying the
        // client, and key interception is gated on the focused surface
        // (keybinds::shortcuts_inhibited).
        use smithay::wayland::seat::WaylandFocus;
        let focused = self
            .keyboard
            .current_focus()
            .and_then(|focus| focus.wl_surface().map(|surface| surface.into_owned()));
        if focused.as_ref() == Some(inhibitor.wl_surface()) && !inhibitor.is_active() {
            inhibitor.activate();
        }
    }
}
smithay::delegate_keyboard_shortcuts_inhibit!(Villain);

impl OutputHandler for Villain {}

impl Villain {
    fn constrained_popup_geometry(
        &self,
        surface: &PopupSurface,
        positioner: PositionerState,
    ) -> smithay::utils::Rectangle<i32, smithay::utils::Logical> {
        let popup = PopupKind::Xdg(surface.clone());
        let fallback = || safe_popup_geometry_unconstrained(positioner).unwrap_or_default();
        let Ok(root) = find_popup_root_surface(&popup) else {
            return fallback();
        };

        // Sum ancestor popup positions with widened arithmetic: the Smithay
        // helper uses i32 addition, while client supplied geometry is untrusted.
        let mut parent = surface.get_parent_surface();
        let mut popup_x = 0_i64;
        let mut popup_y = 0_i64;
        while let Some(parent_surface) = parent {
            if get_role(&parent_surface) != Some(XDG_POPUP_ROLE) {
                break;
            }
            let Some((loc, next_parent)) = with_states(&parent_surface, |states| {
                let attributes = states.data_map.get::<XdgPopupSurfaceData>()?.lock().ok()?;
                Some((attributes.current.geometry.loc, attributes.parent.clone()?))
            }) else {
                return fallback();
            };
            popup_x += i64::from(loc.x);
            popup_y += i64::from(loc.y);
            parent = Some(next_parent);
        }

        // A layer-shell root is not in Space. Its layer geometry includes the
        // full surface-tree bounding box, so remove that offset to recover the
        // root surface's origin (also when a subsurface extends left/up).
        let root_origin = self
            .window_for_surface(&root)
            .and_then(|window| {
                self.space
                    .element_geometry(&window)
                    .map(|geometry| geometry.loc)
            })
            .map(|loc| (i64::from(loc.x), i64::from(loc.y)))
            .or_else(|| {
                self.shell_surfaces
                    .iter()
                    .find(|entry| entry.mapped && entry.layer.wl_surface() == &root)
                    .and_then(|entry| {
                        let map = layer_map_for_output(&entry.output);
                        let geometry = map.layer_geometry(&entry.layer)?;
                        let bbox = entry.layer.bbox();
                        Some((
                            i64::from(geometry.loc.x) - i64::from(bbox.loc.x),
                            i64::from(geometry.loc.y) - i64::from(bbox.loc.y),
                        ))
                    })
            });
        let Some((root_x, root_y)) = root_origin else {
            return fallback();
        };
        let parent_origin = (root_x + popup_x, root_y + popup_y);
        let anchor = positioner.anchor_rect;
        let anchor_center = (
            (parent_origin.0 + i64::from(anchor.loc.x) + i64::from(anchor.size.w) / 2) as f64,
            (parent_origin.1 + i64::from(anchor.loc.y) + i64::from(anchor.size.h) / 2) as f64,
        );
        let output = self
            .space
            .output_under(anchor_center)
            .next()
            .or_else(|| self.space.outputs().next());
        let Some(output_bounds) = output.and_then(|output| self.space.output_geometry(output))
        else {
            return fallback();
        };
        let target = [
            i64::from(output_bounds.loc.x) - parent_origin.0,
            i64::from(output_bounds.loc.y) - parent_origin.1,
            i64::from(output_bounds.size.w),
            i64::from(output_bounds.size.h),
        ];
        safe_popup_geometry(positioner, Some(target)).unwrap_or_else(fallback)
    }
}

impl XdgShellHandler for Villain {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn title_changed(&mut self, _surface: ToplevelSurface) {
        self.desktop_state_dirty = true;
    }

    fn app_id_changed(&mut self, _surface: ToplevelSurface) {
        self.desktop_state_dirty = true;
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        tracing::info!("client created a toplevel surface");
        self.add_window(surface);
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.remove_window(&surface);
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            self.set_window_maximized(&window, Self::window_opening_complete(&window));
        }
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            self.set_window_maximized(&window, false);
        }
    }

    fn fullscreen_request(
        &mut self,
        surface: ToplevelSurface,
        output: Option<smithay::reexports::wayland_server::protocol::wl_output::WlOutput>,
    ) {
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            surface.with_pending_state(|state| state.fullscreen_output = output);
            self.set_window_fullscreen(&window, true);
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            self.set_window_fullscreen(&window, false);
        }
    }

    fn parent_changed(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            self.refresh_window_hints(&window);
        }
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial) {
        if self.seat.owns(&seat)
            && let Some(window) = self.window_for_surface(surface.wl_surface())
        {
            self.start_window_grab(window, Some(serial), None);
        }
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        seat: wl_seat::WlSeat,
        serial: Serial,
        edges: smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge,
    ) {
        if self.seat.owns(&seat)
            && let Some(edges) = crate::window_grab::ResizeEdges::from_xdg(edges)
            && let Some(window) = self.window_for_surface(surface.wl_surface())
        {
            self.start_window_grab(window, Some(serial), Some(edges));
        }
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            self.apply_window_action(&window, WindowAction::Minimize);
        }
    }

    fn new_popup(&mut self, surface: PopupSurface, positioner: PositionerState) {
        let geometry = self.constrained_popup_geometry(&surface, positioner);
        surface.with_pending_state(|state| {
            state.geometry = geometry;
            state.positioner = positioner;
        });
        let _ = self.popups.track_popup(surface.into());
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        let geometry = self.constrained_popup_geometry(&surface, positioner);
        surface.with_pending_state(|state| {
            state.geometry = geometry;
            state.positioner = positioner;
        });
        surface.send_repositioned(token);
    }

    fn popup_destroyed(&mut self, _surface: PopupSurface) {
        self.popups.cleanup();
        self.refresh_pointer(0);
    }

    fn grab(&mut self, surface: PopupSurface, seat: wl_seat::WlSeat, serial: Serial) {
        use smithay::desktop::{
            PopupKeyboardGrab, PopupKind, PopupPointerGrab, find_popup_root_surface,
        };
        use smithay::wayland::seat::WaylandFocus;
        if !self.seat.owns(&seat) {
            return;
        }
        let popup = PopupKind::Xdg(surface.clone());
        let Ok(root) = find_popup_root_surface(&popup) else {
            return;
        };
        // Layer popup grabs may only originate from the focused layer client.
        // Ordinary application popup policy remains separate.
        if !self.layer_surface_visible(&root) {
            return;
        }
        let focused = self
            .keyboard
            .current_focus()
            .and_then(|focus| focus.wl_surface().map(|s| s.into_owned()));
        if focused.as_ref() != Some(&root)
            && focused.as_ref() != surface.get_parent_surface().as_ref()
        {
            return;
        }
        if !self.pointer.has_grab(serial) && !self.keyboard.has_grab(serial) {
            return;
        }
        if let Ok(grab) = self.popups.grab_popup(
            crate::focus::KeyboardFocus::Wayland(root),
            popup,
            &self.seat,
            serial,
        ) {
            self.keyboard
                .clone()
                .set_focus(self, grab.current_grab(), serial);
            self.keyboard
                .clone()
                .set_grab(self, PopupKeyboardGrab::new(&grab), serial);
            self.pointer.clone().set_grab(
                self,
                PopupPointerGrab::new(&grab),
                serial,
                smithay::input::pointer::Focus::Keep,
            );
        }
    }
}

// Smithay's delegation macro connects the protocol objects to the handlers
// above. It is intentionally at the bottom: the implementations are easier to
// find before the generated dispatch glue.
smithay::delegate_compositor!(Villain);
smithay::delegate_cursor_shape!(Villain);
smithay::delegate_data_device!(Villain);
smithay::delegate_data_control!(Villain);
smithay::delegate_ext_data_control!(Villain);
smithay::delegate_output!(Villain);
smithay::delegate_fractional_scale!(Villain);
smithay::delegate_viewporter!(Villain);
smithay::delegate_primary_selection!(Villain);
smithay::delegate_seat!(Villain);
smithay::reexports::wayland_server::delegate_global_dispatch!(Villain: [
    ZwpVirtualKeyboardManagerV1: VirtualKeyboardManagerGlobalData
] => VirtualKeyboardManagerState);
impl Dispatch<ZwpVirtualKeyboardManagerV1, ()> for Villain {
    /// Reject manager requests while virtual-keyboard clients are disallowed.
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &ZwpVirtualKeyboardManagerV1,
        request: zwp_virtual_keyboard_manager_v1::Request,
        data: &(),
        display: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if !state.virtual_keyboard_allowed.load(Ordering::Relaxed) {
            let id = match request {
                zwp_virtual_keyboard_manager_v1::Request::CreateVirtualKeyboard { id, .. } => id,
                _ => return,
            };
            data_init.post_error(
                id,
                zwp_virtual_keyboard_manager_v1::Error::Unauthorized,
                "virtual keyboards are disabled by compositor policy",
            );
            return;
        }

        <VirtualKeyboardManagerState as Dispatch<ZwpVirtualKeyboardManagerV1, (), Self>>::request(
            state, client, resource, request, data, display, data_init,
        );
    }
}

impl Dispatch<ZwpVirtualKeyboardV1, VirtualKeyboardUserData<Villain>> for Villain {
    /// Ignore input from previously created keyboards while the policy is disabled.
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &ZwpVirtualKeyboardV1,
        request: zwp_virtual_keyboard_v1::Request,
        data: &VirtualKeyboardUserData<Self>,
        display: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if state.virtual_keyboard_allowed.load(Ordering::Relaxed)
            || matches!(&request, zwp_virtual_keyboard_v1::Request::Destroy)
        {
            <VirtualKeyboardManagerState as Dispatch<
                ZwpVirtualKeyboardV1,
                VirtualKeyboardUserData<Self>,
                Self,
            >>::request(state, client, resource, request, data, display, data_init);
        }
    }

    /// Forward keyboard object cleanup to Smithay.
    fn destroyed(
        state: &mut Self,
        client_id: ClientId,
        resource: &ZwpVirtualKeyboardV1,
        data: &VirtualKeyboardUserData<Self>,
    ) {
        <VirtualKeyboardManagerState as Dispatch<
            ZwpVirtualKeyboardV1,
            VirtualKeyboardUserData<Self>,
            Self,
        >>::destroyed(state, client_id, resource, data);
    }
}
smithay::delegate_pointer_gestures!(Villain);
smithay::delegate_shm!(Villain);
smithay::delegate_xdg_shell!(Villain);

impl smithay::wayland::dmabuf::DmabufHandler for Villain {
    fn dmabuf_state(&mut self) -> &mut smithay::wayland::dmabuf::DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(
        &mut self,
        _global: &smithay::wayland::dmabuf::DmabufGlobal,
        dmabuf: smithay::backend::allocator::dmabuf::Dmabuf,
        notifier: smithay::wayland::dmabuf::ImportNotifier,
    ) {
        use smithay::backend::renderer::ImportDma;
        if self
            .tty
            .as_mut()
            .is_some_and(|tty| tty.renderer.import_dmabuf(&dmabuf, None).is_ok())
        {
            let _ = notifier.successful::<Self>();
        } else {
            notifier.failed();
        }
    }
}
smithay::delegate_dmabuf!(Villain);

#[cfg(test)]
mod popup_geometry_tests {
    use super::*;
    use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_positioner::{
        Anchor, ConstraintAdjustment, Gravity,
    };

    #[test]
    fn safe_geometry_matches_smithay_for_normal_positioners() {
        let positioner = PositionerState {
            rect_size: (220, 160).into(),
            anchor_rect: smithay::utils::Rectangle::new((90, 80).into(), (20, 12).into()),
            anchor_edges: Anchor::BottomRight,
            gravity: Gravity::BottomRight,
            offset: (8, 6).into(),
            constraint_adjustment: ConstraintAdjustment::FlipX
                | ConstraintAdjustment::FlipY
                | ConstraintAdjustment::SlideX
                | ConstraintAdjustment::SlideY
                | ConstraintAdjustment::ResizeX
                | ConstraintAdjustment::ResizeY,
            ..PositionerState::default()
        };
        let target = smithay::utils::Rectangle::new((-40, -30).into(), (200, 180).into());
        assert_eq!(
            safe_popup_geometry(
                positioner,
                Some([
                    i64::from(target.loc.x),
                    i64::from(target.loc.y),
                    i64::from(target.size.w),
                    i64::from(target.size.h),
                ])
            ),
            Some(positioner.get_unconstrained_geometry(target))
        );
    }

    #[test]
    fn extreme_positioner_values_are_rejected_without_overflow() {
        let positioner = PositionerState {
            rect_size: (i32::MAX, i32::MAX).into(),
            anchor_rect: smithay::utils::Rectangle::new(
                (i32::MAX, i32::MAX).into(),
                (i32::MAX, i32::MAX).into(),
            ),
            anchor_edges: Anchor::BottomRight,
            gravity: Gravity::Left,
            offset: (i32::MAX, i32::MAX).into(),
            ..PositionerState::default()
        };
        assert!(safe_popup_geometry_unconstrained(positioner).is_none());
    }
}
