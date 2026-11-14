//! Output-owned desktop surfaces, independent of workspace membership.
use crate::{focus::KeyboardFocus, state::Villain};
use smithay::{
    backend::renderer::utils::with_renderer_surface_state,
    desktop::{LayerSurface, WindowSurfaceType, layer_map_for_output},
    output::Output,
    reexports::wayland_server::protocol::{wl_output::WlOutput, wl_surface::WlSurface},
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER},
    wayland::shell::wlr_layer::{
        self, KeyboardInteractivity, Layer, WlrLayerShellHandler, WlrLayerShellState,
    },
};

pub struct ShellSurface {
    pub layer: LayerSurface,
    pub output: Output,
    pub mapped: bool,
    pub hide_pending: bool,
}

impl WlrLayerShellHandler for Villain {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: wlr_layer::LayerSurface,
        output: Option<WlOutput>,
        layer: Layer,
        namespace: String,
    ) {
        tracing::info!(%namespace, ?layer, "new layer surface");
        let output = output
            .and_then(|o| Output::from_resource(&o))
            .filter(|o| self.space.outputs().any(|candidate| candidate == o))
            .or_else(|| self.space.outputs().next().cloned());
        let Some(output) = output else {
            surface.send_close();
            return;
        };
        self.shell_surfaces.push(ShellSurface {
            layer: LayerSurface::new(surface, namespace),
            output,
            mapped: false,
            hide_pending: false,
        });
    }

    fn layer_destroyed(&mut self, surface: wlr_layer::LayerSurface) {
        let was_overview = self.shell_surfaces.iter().any(|entry| {
            entry.layer.layer_surface() == &surface
                && entry.layer.namespace() == "knave-shell-overview"
        });
        let had_keyboard_focus = self.keyboard.current_focus()
            == Some(KeyboardFocus::Wayland(surface.wl_surface().clone()));
        self.dismiss_layer_popups(surface.wl_surface());
        if was_overview {
            self.overview_panes.clear();
            self.pending_overview_panes = None;
            self.overview_visible = false;
            self.desktop_state_dirty = true;
            self.request_repaint();
        }
        self.shell_surfaces.retain(|entry| {
            if entry.layer.layer_surface() == &surface {
                layer_map_for_output(&entry.output).unmap_layer(&entry.layer);
                false
            } else {
                true
            }
        });
        self.relayout_active_workspace();
        if had_keyboard_focus {
            self.restore_active_workspace_focus();
        }
    }
}

impl Villain {
    pub fn layer_commit(&mut self, surface: &WlSurface) -> bool {
        let Some(index) = self
            .shell_surfaces
            .iter()
            .position(|e| e.layer.wl_surface() == surface)
        else {
            return false;
        };
        let old_area = self.usable_area();
        let entry = &mut self.shell_surfaces[index];
        let mapped =
            with_renderer_surface_state(surface, |s| s.buffer().is_some()).unwrap_or(false);
        let was_mapped = entry.mapped;
        {
            let mut map = layer_map_for_output(&entry.output);
            if was_mapped && !mapped {
                map.unmap_layer(&entry.layer);
            } else {
                // Arrange even the initial bufferless commit to negotiate its size.
                map.map_layer(&entry.layer)
                    .expect("layer belongs to its assigned output");
                map.arrange();
                if !smithay::wayland::compositor::with_states(surface, |states| {
                    states
                        .data_map
                        .get::<wlr_layer::LayerSurfaceData>()
                        .unwrap()
                        .lock()
                        .unwrap()
                        .initial_configure_sent
                }) {
                    entry.layer.layer_surface().send_configure();
                }
                if !mapped {
                    map.unmap_layer(&entry.layer);
                }
            }
        }
        entry.mapped = mapped;
        if mapped
            && entry.layer.namespace() == "knave-shell-overview"
            && let Some(panes) = self.pending_overview_panes.take()
        {
            self.overview_panes = panes;
        }
        if !mapped && entry.layer.namespace() == "knave-shell-overview" {
            self.overview_panes.clear();
            self.pending_overview_panes = None;
            // A late unmap can acknowledge an earlier hide after Super reopened it.
            let expected_hide = std::mem::take(&mut entry.hide_pending);
            if was_mapped && !expected_hide && self.overview_visible {
                self.overview_visible = false;
                self.desktop_state_dirty = true;
            }
        }
        if was_mapped && !mapped {
            self.dismiss_layer_popups(surface);
        }
        if mapped != was_mapped || self.usable_area() != old_area {
            self.relayout_active_workspace();
        } else {
            self.refresh_pointer(0);
        }
        if !self.keyboard.is_grabbed()
            && let Some(focus) = self.exclusive_layer_focus()
            && self.keyboard.current_focus().as_ref() != Some(&focus)
        {
            self.release_pointer_buttons();
            self.focus_layer(focus);
        } else if was_mapped
            && !mapped
            && self.keyboard.current_focus() == Some(KeyboardFocus::Wayland(surface.clone()))
        {
            self.restore_active_workspace_focus();
        }
        self.request_repaint();
        true
    }

    fn dismiss_layer_popups(&mut self, surface: &WlSurface) {
        for (popup, _) in smithay::desktop::PopupManager::popups_for_surface(surface) {
            let _ = smithay::desktop::PopupManager::dismiss_popup(surface, &popup);
        }
        if self
            .keyboard
            .grab_start_data()
            .is_some_and(|start| start.focus == Some(KeyboardFocus::Wayland(surface.clone())))
        {
            self.keyboard.clone().unset_grab(self);
            self.release_pointer_buttons();
        }
    }

    pub fn arrange_layers(&mut self) {
        for output in self.space.outputs() {
            layer_map_for_output(output).arrange();
        }
    }

    pub fn usable_area(&self) -> Rectangle<i32, Logical> {
        let mut area = self
            .space
            .outputs()
            .next()
            .map(|o| layer_map_for_output(o).non_exclusive_zone())
            .unwrap_or_else(|| Rectangle::from_size(self.output_size));
        area.size.w = area.size.w.max(1);
        area.size.h = area.size.h.max(1);
        area
    }

    pub(crate) fn overview_mapped(&self) -> bool {
        self.shell_surfaces
            .iter()
            .any(|entry| entry.mapped && entry.layer.namespace() == "knave-shell-overview")
    }

    pub fn layer_surface_visible(&self, surface: &WlSurface) -> bool {
        let fullscreen_hides_bar = self.workspace_has_fullscreen(self.active_workspace);
        let hidden = |entry: &ShellSurface| {
            fullscreen_hides_bar
                && (entry.layer.layer() == Layer::Top
                    || entry.layer.namespace() == "knave-shell-bar")
        };

        // Rendering and hit-testing call this while iterating a locked LayerMap.
        // Check root surfaces before asking LayerMap about descendants so that
        // the common path never tries to lock that same map recursively.
        if let Some(entry) = self
            .shell_surfaces
            .iter()
            .find(|entry| entry.mapped && entry.layer.wl_surface() == surface)
        {
            return !hidden(entry);
        }

        self.shell_surfaces.iter().any(|entry| {
            entry.mapped
                && !hidden(entry)
                && layer_map_for_output(&entry.output)
                    .layer_for_surface(surface, WindowSurfaceType::ALL)
                    .is_some()
        })
    }

    pub fn exclusive_layer_focus(&self) -> Option<KeyboardFocus> {
        if !self.host_focused {
            return None;
        }
        for kind in [Layer::Overlay, Layer::Top] {
            if let Some(entry) = self.shell_surfaces.iter().rev().find(|e| {
                e.mapped
                    && self.layer_surface_visible(e.layer.wl_surface())
                    && e.layer.layer() == kind
                    && e.layer.cached_state().keyboard_interactivity
                        == KeyboardInteractivity::Exclusive
            }) {
                return Some(KeyboardFocus::Wayland(entry.layer.wl_surface().clone()));
            }
        }
        None
    }

    pub fn layer_under_pointer(
        &self,
        upper: bool,
    ) -> Option<(WlSurface, Point<f64, Logical>, Option<KeyboardFocus>)> {
        let kinds = if upper {
            [Layer::Overlay, Layer::Top]
        } else {
            [Layer::Bottom, Layer::Background]
        };
        for kind in kinds {
            if kind == Layer::Top && self.workspace_has_fullscreen(self.active_workspace) {
                continue;
            }
            for output in self.space.outputs() {
                let map = layer_map_for_output(output);
                for layer in map.layers_on(kind).rev() {
                    if !self.layer_surface_visible(layer.wl_surface()) {
                        continue;
                    }
                    let origin = map.layer_geometry(layer)?.loc.to_f64();
                    if let Some((surface, offset)) =
                        layer.surface_under(self.pointer_location - origin, WindowSurfaceType::ALL)
                    {
                        let keyboard = layer
                            .can_receive_keyboard_focus()
                            .then(|| KeyboardFocus::Wayland(layer.wl_surface().clone()));
                        return Some((surface, origin + offset.to_f64(), keyboard));
                    }
                }
            }
        }
        None
    }

    pub fn focus_layer(&mut self, focus: KeyboardFocus) {
        if self.keyboard.current_focus().as_ref() != Some(&focus) {
            for window in self.space.elements() {
                Self::set_activated(window, false);
            }
            self.keyboard
                .clone()
                .set_focus(self, Some(focus), SERIAL_COUNTER.next_serial());
        }
    }
}
smithay::delegate_layer_shell!(Villain);

#[cfg(test)]
#[path = "layer_tests.rs"]
mod tests;
