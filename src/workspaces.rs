//! Ordered workspaces with a master-and-stack layout.
use crate::{
    dispatch::{FocusCycleStep, FocusDirection},
    focus::KeyboardFocus,
    state::Villain,
};
use knave_desktop_api::{
    WindowId, WindowSummary as WindowInfo, WorkspaceId, WorkspaceSummary as WorkspaceInfo,
};
use smithay::{
    desktop::{Window, WindowSurfaceType},
    input::pointer::MotionEvent,
    reexports::{wayland_protocols::xdg::shell::server::xdg_toplevel, wayland_server::Resource},
    utils::{Point, Rectangle, SERIAL_COUNTER, Size},
    wayland::{seat::WaylandFocus, shell::xdg::ToplevelSurface},
    xwayland::X11Surface,
};
use std::{
    collections::BTreeMap,
    process::Command,
    time::{Duration, Instant},
};

struct WindowOpeningComplete(Instant);
mod drag_layout;
use drag_layout::weighted_heights;

#[derive(Default)]
pub struct Workspace {
    windows: Vec<WorkspaceWindow>,
    /// Most-recently-focused managed windows, oldest first.
    focus_history: Vec<WindowId>,
    minimized_history: Vec<Window>,
    fullscreen: Option<WindowId>,
    maximized: Option<WindowId>,
    master_ratio: Option<u16>,
}

struct WorkspaceWindow {
    id: WindowId,
    stacking_order: u64,
    window: Window,
    minimized: bool,
    initial_focus_pending: bool,
    parent: Option<WindowId>,
    floating: Option<Rectangle<i32, smithay::utils::Logical>>,
    stack_weight: u32,
    floating_override: Option<bool>,
    stay_on_top: bool,
    center_override: Option<bool>,
    initial_rules_applied: bool,
    saved_floating: Option<Geometry>,
}

#[derive(Clone, Copy)]
pub(crate) enum WindowAction {
    Close,
    Minimize,
}

fn master_stack_layout(
    output: Size<i32, smithay::utils::Logical>,
    window_count: usize,
    ratio: u16,
) -> Vec<(
    Point<i32, smithay::utils::Logical>,
    Size<i32, smithay::utils::Logical>,
)> {
    match window_count {
        0 => Vec::new(),
        1 => vec![((0, 0).into(), output)],
        _ => {
            let master_width = master_width(output.w, ratio);
            let stack_width = output.w - master_width;
            let stack_count = window_count as i32 - 1;
            let stack_height = output.h / stack_count;
            let mut result = Vec::with_capacity(window_count);
            result.push(((0, 0).into(), (master_width, output.h).into()));
            for index in 0..stack_count {
                let y = index * stack_height;
                let height = if index + 1 == stack_count {
                    output.h - y
                } else {
                    stack_height
                };
                result.push(((master_width, y).into(), (stack_width, height).into()));
            }
            result
        }
    }
}

pub(crate) fn master_width(width: i32, ratio: u16) -> i32 {
    ((i64::from(width) * i64::from(ratio.clamp(1000, 9000))) / 10000) as i32
}

fn center_of(rect: &Rectangle<i32, smithay::utils::Logical>) -> (f64, f64) {
    (
        f64::from(rect.loc.x) + f64::from(rect.size.w) / 2.0,
        f64::from(rect.loc.y) + f64::from(rect.size.h) / 2.0,
    )
}

/// Pick the candidate whose center is nearest to `origin` while requiring the
/// candidate center to lie strictly in `direction`. First match wins ties, so
/// layout order breaks distance dead heats deterministically.
fn nearest_center_in_direction(
    centers: &[(f64, f64)],
    origin: (f64, f64),
    direction: FocusDirection,
) -> Option<usize> {
    let in_direction = |center: (f64, f64)| match direction {
        FocusDirection::Left => center.0 < origin.0,
        FocusDirection::Right => center.0 > origin.0,
        FocusDirection::Up => center.1 < origin.1,
        FocusDirection::Down => center.1 > origin.1,
    };
    let mut best: Option<(f64, usize)> = None;
    for (index, &center) in centers.iter().enumerate() {
        if !in_direction(center) {
            continue;
        }
        let distance = (center.0 - origin.0).powi(2) + (center.1 - origin.1).powi(2);
        if best.is_none_or(|(closest, _)| distance < closest) {
            best = Some((distance, index));
        }
    }
    best.map(|(_, index)| index)
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SplitContext {
    pub workspace: usize,
    pub area: Geometry,
    windows: Vec<WindowId>,
}

type Geometry = Rectangle<i32, smithay::utils::Logical>;
type WindowPlacement = (Window, Geometry, bool, bool, bool, bool);

pub(crate) struct WorkspacePreviewScene {
    pub windows: Vec<(Window, Geometry)>,
}

fn fixed_size(
    min: Size<i32, smithay::utils::Logical>,
    max: Size<i32, smithay::utils::Logical>,
) -> bool {
    min.w > 0 && min.h > 0 && min == max
}

pub(crate) fn constrain_geometry(
    mut rect: Geometry,
    output: Size<i32, smithay::utils::Logical>,
    min: Size<i32, smithay::utils::Logical>,
    max: Size<i32, smithay::utils::Logical>,
) -> Geometry {
    let limit = |size: i32, min: i32, max: i32, output: i32| {
        let upper = if max > 0 { max.min(output) } else { output }.max(1);
        size.clamp(min.max(1).min(upper), upper)
    };
    rect.size.w = limit(rect.size.w, min.w, max.w, output.w);
    rect.size.h = limit(rect.size.h, min.h, max.h, output.h);
    rect.loc.x = rect.loc.x.clamp(0, (output.w - rect.size.w).max(0));
    rect.loc.y = rect.loc.y.clamp(0, (output.h - rect.size.h).max(0));
    rect
}

fn move_floating_geometry(
    mut saved: Geometry,
    visible: Geometry,
    output: Size<i32, smithay::utils::Logical>,
    min: Size<i32, smithay::utils::Logical>,
    max: Size<i32, smithay::utils::Logical>,
    delta: (i32, i32),
) -> Geometry {
    let mut moved = visible;
    moved.loc.x = moved.loc.x.saturating_add(delta.0);
    moved.loc.y = moved.loc.y.saturating_add(delta.1);
    let loc = constrain_geometry(moved, output, min, max).loc;
    if delta.0 != 0 {
        saved.loc.x = loc.x;
    }
    if delta.1 != 0 {
        saved.loc.y = loc.y;
    }
    saved
}

fn window_identity(window: &Window) -> Option<(String, String)> {
    if let Some(surface) = window.toplevel() {
        return Some(smithay::wayland::compositor::with_states(
            surface.wl_surface(),
            |states| {
                let attributes = states
                    .data_map
                    .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                    .expect("XDG toplevel data")
                    .lock()
                    .unwrap();
                (
                    attributes.app_id.clone().unwrap_or_default(),
                    attributes.title.clone().unwrap_or_default(),
                )
            },
        ));
    }
    window
        .x11_surface()
        .map(|surface| (surface.class(), surface.title()))
}

impl Workspace {
    fn stacking_path(&self, window: &Window) -> Vec<(bool, u64, u64)> {
        let mut entry = self.windows.iter().find(|entry| entry.window == *window);
        let mut path = Vec::new();
        // Parent hints are client-controlled; bound traversal even for cycles.
        for _ in 0..self.windows.len() {
            let Some(current) = entry.filter(|entry| entry.floating.is_some()) else {
                break;
            };
            path.push((current.stay_on_top, current.stacking_order, current.id.0));
            entry = current
                .parent
                .and_then(|id| self.windows.iter().find(|e| e.id == id));
        }
        path.reverse();
        path
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    fn descendant_of(&self, entry: &WorkspaceWindow, ancestor: WindowId) -> bool {
        let mut parent = entry.parent;
        for _ in 0..self.windows.len() {
            let Some(id) = parent else {
                break;
            };
            if id == ancestor {
                return true;
            }
            parent = self
                .windows
                .iter()
                .find(|entry| entry.id == id)
                .and_then(|entry| entry.parent);
        }
        false
    }

    fn hidden_by_parent(&self, entry: &WorkspaceWindow) -> bool {
        self.windows
            .iter()
            .any(|parent| parent.minimized && self.descendant_of(entry, parent.id))
    }
}

impl Villain {
    pub(crate) fn capture_window(&self, id: WindowId) -> Option<Window> {
        self.workspaces
            .iter()
            .flat_map(|workspace| workspace.windows.iter())
            .find(|entry| entry.id == id)
            .map(|entry| entry.window.clone())
    }
    pub(crate) fn master_override(&self) -> Option<u16> {
        self.workspaces[self.active_workspace].master_ratio
    }
    pub(crate) fn master_ratio(&self, index: usize) -> u16 {
        self.workspaces[index]
            .master_ratio
            .unwrap_or(u16::from(self.config.master_percent) * 100)
    }

    pub(crate) fn split_context(&self) -> Option<SplitContext> {
        let area = if self.config.appearance.views.tiled.gaps {
            crate::appearance::inset(self.usable_area(), self.config.appearance.gaps.outer)
        } else {
            self.usable_area()
        };
        if area.size.w < 10 || area.size.h < 1 {
            return None;
        }
        let workspace = &self.workspaces[self.active_workspace];
        if [workspace.fullscreen, workspace.maximized]
            .into_iter()
            .flatten()
            .any(|id| {
                workspace.windows.iter().any(|entry| {
                    entry.id == id && !entry.minimized && !workspace.hidden_by_parent(entry)
                })
            })
        {
            return None;
        }
        let windows: Vec<_> = self.workspaces[self.active_workspace]
            .windows
            .iter()
            .filter(|entry| {
                !entry.minimized && !workspace.hidden_by_parent(entry) && entry.floating.is_none()
            })
            .map(|entry| entry.id)
            .collect();
        (windows.len() >= 2).then_some(SplitContext {
            workspace: self.active_workspace,
            area,
            windows,
        })
    }

    pub(crate) fn split_under_pointer(&self) -> Option<SplitContext> {
        if !self.host_focused
            || self.pointer.is_grabbed()
            || self.keyboard.is_grabbed()
            || self.exclusive_layer_focus().is_some()
            || self.layer_under_pointer(true).is_some()
        {
            return None;
        }
        let area = if self.config.appearance.views.tiled.gaps {
            crate::appearance::inset(self.usable_area(), self.config.appearance.gaps.outer)
        } else {
            self.usable_area()
        };
        let x = area.loc.x + master_width(area.size.w, self.master_ratio(self.active_workspace));
        if (self.pointer_location.x - f64::from(x)).abs() > 5.0
            || !area.to_f64().contains(self.pointer_location)
        {
            return None;
        }
        let context = self.split_context()?;
        // Dialogs, popups and override-redirect windows keep their normal input.
        let Some((window, origin)) = self.space.element_under(self.pointer_location) else {
            return Some(context);
        };
        let entry = self.workspaces[self.active_workspace]
            .windows
            .iter()
            .find(|entry| &entry.window == window)?;
        if entry.floating.is_some() {
            return None;
        }
        let (surface, _) = window.surface_under(
            self.pointer_location - origin.to_f64(),
            WindowSurfaceType::ALL,
        )?;
        let root =
            std::iter::successors(Some(surface), smithay::wayland::compositor::get_parent).last();
        if root != Self::window_surface(window) {
            return None;
        }
        Some(context)
    }

    /// May run inside a pointer grab: do not re-enter pointer dispatch here.
    pub(crate) fn apply_master_ratio(&mut self, ratio: Option<u16>) {
        let before = self.master_ratio(self.active_workspace);
        self.workspaces[self.active_workspace].master_ratio =
            ratio.map(|value| value.clamp(1000, 9000));
        if self.master_ratio(self.active_workspace) != before {
            self.map_active_workspace();
            self.request_repaint();
        }
    }

    pub(crate) fn resize_master(&mut self, delta: Option<i16>) {
        if self.split_context().is_none() {
            return;
        }
        self.release_pointer_buttons();
        let ratio = delta.map(|delta| {
            (i32::from(self.master_ratio(self.active_workspace)) + i32::from(delta) * 100)
                .clamp(1000, 9000) as u16
        });
        self.apply_master_ratio(ratio);
        self.refresh_pointer_surface(0);
    }

    pub(crate) fn window_constraints(
        window: &Window,
    ) -> (
        Size<i32, smithay::utils::Logical>,
        Size<i32, smithay::utils::Logical>,
    ) {
        if let Some(surface) = window.toplevel() {
            smithay::wayland::compositor::with_states(surface.wl_surface(), |states| {
                let mut guard = states
                    .cached_state
                    .get::<smithay::wayland::shell::xdg::SurfaceCachedState>();
                let state = guard.current();
                (state.min_size, state.max_size)
            })
        } else if let Some(surface) = window.x11_surface() {
            (
                surface.min_size().unwrap_or_default(),
                surface.max_size().unwrap_or_default(),
            )
        } else {
            (Size::default(), Size::default())
        }
    }

    /// Apply client hints on map and when committed hints/parent relationships change.
    pub fn refresh_window_hints(&mut self, window: &Window) {
        let parent_window = if let Some(surface) = window.toplevel() {
            surface
                .parent()
                .and_then(|surface| self.window_for_surface(&surface))
        } else {
            window
                .x11_surface()
                .and_then(|surface| surface.is_transient_for())
                .and_then(|id| self.window_for_x11_id(id))
        }
        .filter(|parent| parent != window);
        let parent = parent_window.as_ref().and_then(|parent| {
            self.workspaces
                .iter()
                .enumerate()
                .find_map(|(index, workspace)| {
                    workspace
                        .windows
                        .iter()
                        .find(|entry| entry.window == *parent)
                        .map(|entry| (index, entry.id))
                })
        });
        let Some((mut index, position)) =
            self.workspaces
                .iter()
                .enumerate()
                .find_map(|(index, workspace)| {
                    workspace
                        .windows
                        .iter()
                        .position(|entry| entry.window == *window)
                        .map(|pos| (index, pos))
                })
        else {
            return;
        };
        let mut changed = false;
        let mut moved_from = None;
        if let Some((target, _)) = parent
            && index != target
        {
            let entry = self.workspaces[index].windows.remove(position);
            self.workspaces[index]
                .minimized_history
                .retain(|candidate| candidate != window);
            if self.workspaces[index].fullscreen == Some(entry.id) {
                self.workspaces[index].fullscreen = None;
                self.workspaces[target].fullscreen = Some(entry.id);
            }
            if self.workspaces[index].maximized == Some(entry.id) {
                self.workspaces[index].maximized = None;
                self.workspaces[target].maximized = Some(entry.id);
            }
            if entry.minimized {
                self.workspaces[target]
                    .minimized_history
                    .push(window.clone());
            }
            self.workspaces[target].windows.push(entry);
            moved_from = Some(index);
            index = target;
            changed = true;
        }
        let (min, max) = Self::window_constraints(window);
        let floating = parent.is_some()
            || fixed_size(min, max)
            || window.x11_surface().is_some_and(|surface| {
                surface.window_type() == Some(smithay::xwayland::xwm::WmWindowType::Dialog)
            });
        let parent_rect = parent_window
            .as_ref()
            .and_then(|parent| {
                self.workspace_layout(index)
                    .into_iter()
                    .find(|(window, _, _, _, _, _)| window == parent)
                    .map(|(_, rect, _, _, _, _)| rect)
            })
            .unwrap_or_else(|| Rectangle::from_size(self.output_size));
        let entry = self.workspaces[index]
            .windows
            .iter_mut()
            .find(|entry| entry.window == *window)
            .unwrap();
        let parent_id = parent.map(|(_, id)| id);
        changed |= entry.parent != parent_id;
        entry.parent = parent_id;
        let floating = entry.floating_override.unwrap_or(floating);
        let geometry = if floating {
            let geometry = entry.floating.unwrap_or_else(|| {
                let natural = window.geometry().size;
                let size = if fixed_size(min, max) {
                    min
                } else if natural.w > 0 && natural.h > 0 {
                    natural
                } else {
                    (600, 400).into()
                };
                let size =
                    constrain_geometry(Rectangle::from_size(size), self.output_size, min, max).size;
                let location = if entry.center_override == Some(false) {
                    parent_rect.loc
                } else {
                    (
                        parent_rect.loc.x + (parent_rect.size.w - size.w) / 2,
                        parent_rect.loc.y + (parent_rect.size.h - size.h) / 2,
                    )
                        .into()
                };
                Rectangle::new(location, size)
            });
            Some(constrain_geometry(geometry, self.output_size, min, max))
        } else {
            None
        };
        changed |= entry.floating != geometry;
        entry.floating = geometry;
        if moved_from.is_some() {
            self.trim_dynamic_workspaces();
        }
        if changed {
            self.desktop_state_dirty = true;
            if index == self.active_workspace || moved_from == Some(self.active_workspace) {
                self.relayout_active_workspace();
            }
            if index != self.active_workspace {
                self.configure_workspace(index);
            }
        }
    }

    pub(crate) fn workspace_has_fullscreen(&self, index: usize) -> bool {
        let workspace = &self.workspaces[index];
        workspace.fullscreen.is_some_and(|id| {
            workspace.windows.iter().any(|entry| {
                entry.id == id && !entry.minimized && !workspace.hidden_by_parent(entry)
            })
        })
    }

    pub(crate) fn workspace_layout(&self, index: usize) -> Vec<WindowPlacement> {
        let workspace = &self.workspaces[index];
        let fullscreen = workspace.fullscreen.filter(|id| {
            workspace.windows.iter().any(|entry| {
                entry.id == *id && !entry.minimized && !workspace.hidden_by_parent(entry)
            })
        });
        let maximized = workspace.maximized.filter(|id| {
            workspace.windows.iter().any(|entry| {
                entry.id == *id && !entry.minimized && !workspace.hidden_by_parent(entry)
            })
        });
        let exclusive = fullscreen.or(maximized);
        let count = workspace
            .windows
            .iter()
            .filter(|entry| {
                !entry.minimized && !workspace.hidden_by_parent(entry) && entry.floating.is_none()
            })
            .count();
        let usable = self.usable_area();
        let area = if self.config.appearance.views.tiled.gaps {
            crate::appearance::inset(usable, self.config.appearance.gaps.outer)
        } else {
            usable
        };
        let weights: Vec<_> = workspace
            .windows
            .iter()
            .filter(|entry| {
                !entry.minimized && !workspace.hidden_by_parent(entry) && entry.floating.is_none()
            })
            .skip(1)
            .map(|entry| entry.stack_weight)
            .collect();
        let mut layout = master_stack_layout(area.size, count, self.master_ratio(index));
        let heights = weighted_heights(area.size.h, &weights);
        let mut y = 0;
        for (tile, height) in layout.iter_mut().skip(1).zip(heights) {
            tile.0.y = y;
            tile.1.h = height;
            y += height;
        }
        let mut tiles = layout.into_iter();
        let mut result = Vec::new();
        for entry in &workspace.windows {
            let is_fullscreen = workspace.fullscreen == Some(entry.id);
            let is_maximized = workspace.maximized == Some(entry.id);
            let base = if let Some(rect) = entry.floating {
                let (min, max) = Self::window_constraints(&entry.window);
                constrain_geometry(rect, self.output_size, min, max)
            } else if !entry.minimized && !workspace.hidden_by_parent(entry) {
                let (loc, size) = tiles.next().unwrap();
                Rectangle::new(loc + area.loc, size)
            } else {
                Rectangle::from_size(self.output_size)
            };
            let rect = if is_fullscreen {
                Rectangle::from_size(self.output_size)
            } else if is_maximized {
                let view = self.config.appearance.views.maximized;
                let rect = if view.gaps {
                    crate::appearance::inset(usable, self.config.appearance.gaps.outer)
                } else {
                    usable
                };
                crate::appearance::client_rect(rect, &self.config.appearance, view, false)
            } else {
                let view = if entry.floating.is_some() {
                    self.config.appearance.views.floating
                } else {
                    self.config.appearance.views.tiled
                };
                if entry.floating.is_some() {
                    // Floating rectangles are saved client geometry; effects never shrink them on every relayout.
                    self.floating_placement(base)
                } else {
                    crate::appearance::client_rect(base, &self.config.appearance, view, true)
                }
            };
            let visible = !entry.minimized
                && !workspace.hidden_by_parent(entry)
                && exclusive.is_none_or(|owner| {
                    owner == entry.id
                        || (entry.floating.is_some() && workspace.descendant_of(entry, owner))
                });
            result.push((
                entry.window.clone(),
                rect,
                is_fullscreen,
                is_maximized,
                entry.floating.is_none(),
                visible,
            ));
        }
        // Map tiles first, then the expanded window, then its floating dialogs.
        result.sort_by_cached_key(|(window, _, fullscreen, maximized, tiled, _)| {
            let layer = if *fullscreen || *maximized {
                1
            } else if *tiled {
                0
            } else {
                2
            };
            (
                layer,
                if layer == 2 {
                    workspace.stacking_path(window)
                } else {
                    Vec::new()
                },
            )
        });
        result
    }

    pub(crate) fn workspace_preview_scene(&self, index: usize) -> Option<WorkspacePreviewScene> {
        (index < self.workspaces.len()).then(|| WorkspacePreviewScene {
            windows: self
                .workspace_layout(index)
                .into_iter()
                .filter_map(|(window, geometry, _, _, _, visible)| {
                    visible.then_some((window, geometry))
                })
                .collect(),
        })
    }

    fn configure_workspace(&self, index: usize) {
        for (window, rect, fullscreen, maximized, tiled, _) in self.workspace_layout(index) {
            Self::configure_window(&window, rect.loc, rect.size, fullscreen, maximized, tiled);
        }
    }

    pub fn set_window_fullscreen(&mut self, window: &Window, fullscreen: bool) {
        self.desktop_state_dirty = true;
        let Some(index) = self.workspace_for_window(window) else {
            return;
        };
        let Some(entry) = self.workspaces[index]
            .windows
            .iter()
            .find(|entry| entry.window == *window)
        else {
            return;
        };
        let id = entry.id;
        if fullscreen == (self.workspaces[index].fullscreen == Some(id)) {
            // xdg-shell requires a configure response even to a repeated request.
            if let Some(surface) = window.toplevel() {
                surface.send_configure();
            }
            return;
        }
        tracing::info!(
            window = id.0,
            workspace = index + 1,
            active_workspace = self.active_workspace + 1,
            fullscreen,
            "window fullscreen state changed"
        );
        if index == self.active_workspace {
            self.release_pointer_buttons();
        }
        if fullscreen {
            self.workspaces[index].fullscreen = Some(id);
        } else if self.workspaces[index].fullscreen == Some(id) {
            self.workspaces[index].fullscreen = None;
        }
        if index == self.active_workspace {
            self.release_pointer_buttons();
            self.relayout_active_workspace();
        } else {
            self.configure_workspace(index);
        }
    }

    pub(crate) fn window_opening_complete(window: &Window) -> bool {
        window
            .user_data()
            .get::<WindowOpeningComplete>()
            .is_some_and(|opened| opened.0.elapsed() >= Duration::from_secs(1))
    }

    pub(crate) fn send_window_frame(
        window: &Window,
        output: &smithay::output::Output,
        now: Duration,
    ) {
        // Some toolkits commit a placeholder buffer before requesting their
        // startup state. Start a short settling period at the first content
        // frame callback; XWayland can draw before application setup finishes.
        if window.user_data().get::<WindowOpeningComplete>().is_none()
            && let Some(surface) = window.wl_surface()
            && smithay::backend::renderer::utils::with_renderer_surface_state(&surface, |state| {
                state.buffer().is_some()
            })
            .unwrap_or(false)
            && smithay::wayland::compositor::with_states(&surface, |states| {
                !states
                    .cached_state
                    .get::<smithay::wayland::compositor::SurfaceAttributes>()
                    .current()
                    .frame_callbacks
                    .is_empty()
            })
        {
            window
                .user_data()
                .insert_if_missing(|| WindowOpeningComplete(Instant::now()));
        }
        window.send_frame(output, now, None, |_, _| Some(output.clone()));
    }

    /// Maximization overlays the layout; tiled order and floating geometry stay intact.
    pub fn set_window_maximized(&mut self, window: &Window, maximized: bool) {
        self.desktop_state_dirty = true;
        let Some(index) = self.workspace_for_window(window) else {
            return;
        };
        let Some(id) = self.window_id(window) else {
            return;
        };
        if maximized == (self.workspaces[index].maximized == Some(id)) {
            if let Some(surface) = window.toplevel() {
                surface.send_configure();
            }
            return;
        }
        if index == self.active_workspace {
            self.release_pointer_buttons();
        }
        if maximized {
            self.workspaces[index].maximized = Some(id);
        } else {
            self.workspaces[index].maximized = None;
        }
        if index == self.active_workspace {
            self.release_pointer_buttons();
            self.relayout_active_workspace();
        } else {
            self.configure_workspace(index);
        }
    }

    pub fn maximize_focused_window(&mut self, maximized: Option<bool>) -> bool {
        let Some(window) = self.focused_window() else {
            return false;
        };
        let workspace = &self.workspaces[self.active_workspace];
        let maximized = maximized.unwrap_or_else(|| workspace.maximized != self.window_id(&window));
        self.set_window_maximized(&window, maximized);
        true
    }

    pub(crate) fn floating_placement(&self, rect: Geometry) -> Geometry {
        let appearance = &self.config.appearance;
        let view = appearance.views.floating;
        if (!view.gaps || appearance.gaps.outer == Default::default())
            && (!view.borders || appearance.border.width == Default::default())
        {
            return rect;
        }
        let usable = self.usable_area();
        let bounds = if view.gaps {
            crate::appearance::inset(usable, appearance.gaps.outer)
        } else {
            usable
        };
        crate::appearance::constrain_floating(rect, bounds, appearance, view)
    }

    pub(crate) fn floating_geometry(&self, window: &Window) -> Option<Geometry> {
        let workspace = &self.workspaces[self.workspace_for_window(window)?];
        workspace
            .windows
            .iter()
            .find(|entry| {
                entry.window == *window
                    && !entry.minimized
                    && workspace.fullscreen != Some(entry.id)
                    && workspace.maximized != Some(entry.id)
            })
            .and_then(|entry| entry.floating)
            .map(|rect| self.floating_placement(rect))
    }

    /// Configure/map a floating window without re-entering pointer dispatch.
    pub(crate) fn set_floating_geometry(&mut self, window: &Window, geometry: Geometry) {
        if self.floating_geometry(window).is_none() {
            return;
        }
        let (min, max) = Self::window_constraints(window);
        let geometry = constrain_geometry(geometry, self.output_size, min, max);
        let index = self.workspace_for_window(window).unwrap();
        let entry = self.workspaces[index]
            .windows
            .iter_mut()
            .find(|entry| entry.window == *window)
            .unwrap();
        entry.floating = Some(geometry);
        let geometry = self.floating_placement(geometry);
        Self::configure_window(window, geometry.loc, geometry.size, false, false, false);
        if self.space.element_location(window).is_some() {
            self.space.map_element(window.clone(), geometry.loc, false);
            self.sync_x11_stacking();
            self.request_repaint();
        }
    }

    pub fn spawn(&mut self, argv: Vec<String>) -> std::io::Result<()> {
        let Some((program, arguments)) = argv.split_first() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "command is empty",
            ));
        };
        self.reap_children();
        let workspace = self.active_workspace;
        let mut command = Command::new(program);
        command
            .args(arguments)
            // A shell started from a desktop terminal can inherit the host's
            // X11 display and a forced GTK backend. Neither describes the
            // session that Villain is providing to this child.
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_SOCKET")
            .envs(self.application_environment());
        let child = command.spawn()?;
        tracing::info!(
            pid = child.id(),
            workspace = workspace + 1,
            ?argv,
            "application launched"
        );
        self.children.push((workspace, child));
        Ok(())
    }
    /// Environment inherited by applications Villain launches into its session.
    pub(crate) fn application_environment(&self) -> BTreeMap<String, String> {
        let mut environment = self.config.environment.clone();
        environment.remove("WAYLAND_SOCKET");
        environment.insert(
            "WAYLAND_DISPLAY".into(),
            self.socket_name.to_string_lossy().into_owned(),
        );
        if let Some(display) = self.xwayland_display {
            environment.insert("DISPLAY".into(), format!(":{display}"));
        } else {
            environment.remove("DISPLAY");
        }
        environment
    }

    pub fn reap_children(&mut self) {
        self.children
            .retain_mut(|(_, child)| match child.try_wait() {
                Ok(Some(status)) => {
                    tracing::info!(pid = child.id(), %status, "launched app exited");
                    false
                }
                Ok(None) => true,
                Err(error) => {
                    tracing::warn!(%error, "could not reap app");
                    true
                }
            });
    }
    fn configure_window(
        window: &Window,
        location: Point<i32, smithay::utils::Logical>,
        size: Size<i32, smithay::utils::Logical>,
        fullscreen: bool,
        maximized: bool,
        tiled: bool,
    ) {
        if let Some(surface) = window.toplevel() {
            surface.with_pending_state(|pending| {
                pending.size = Some(size);
                for state in [
                    xdg_toplevel::State::TiledLeft,
                    xdg_toplevel::State::TiledRight,
                    xdg_toplevel::State::TiledTop,
                    xdg_toplevel::State::TiledBottom,
                ] {
                    if tiled && !fullscreen && !maximized {
                        pending.states.set(state);
                    } else {
                        pending.states.unset(state);
                    }
                }
                if maximized {
                    pending.states.set(xdg_toplevel::State::Maximized);
                } else {
                    pending.states.unset(xdg_toplevel::State::Maximized);
                }
                if fullscreen {
                    pending.states.set(xdg_toplevel::State::Fullscreen);
                } else {
                    pending.states.unset(xdg_toplevel::State::Fullscreen);
                    pending.fullscreen_output = None;
                }
            });
            surface.send_pending_configure();
        } else if let Some(surface) = window.x11_surface() {
            if surface.is_fullscreen() != fullscreen
                && let Err(error) = surface.set_fullscreen(fullscreen)
            {
                tracing::warn!(%error, "could not update X11 fullscreen state");
            }
            if surface.is_maximized() != maximized
                && let Err(error) = surface.set_maximized(maximized)
            {
                tracing::warn!(%error, "could not update X11 maximized state");
            }
            let geometry = Rectangle::new(location, size);
            if surface.geometry() != geometry
                && let Err(error) = surface.configure(geometry)
            {
                tracing::warn!(%error, window = surface.window_id(), "could not configure X11 window");
            }
        }
    }

    fn send_pending_configure(window: &Window) {
        if let Some(surface) = window.toplevel() {
            surface.send_pending_configure();
        }
    }

    /// Update a window's activation state and notify the client only when the
    /// state actually changed. Sending an activation configure for every
    /// window during every relayout creates a burst of competing configure
    /// serials while the workspace is being switched.
    pub(crate) fn set_activated(window: &Window, activated: bool) {
        if window.set_activated(activated) {
            Self::send_pending_configure(window);
        }
    }

    pub(crate) fn window_surface(
        window: &Window,
    ) -> Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface> {
        window.wl_surface().map(|surface| surface.into_owned())
    }

    fn workspace_for_pid(&self, pid: Option<u32>) -> usize {
        self.children
            .iter()
            .find(|(_, child)| Some(child.id()) == pid)
            .map(|(index, _)| *index)
            .unwrap_or(self.active_workspace)
    }

    fn add_workspace_window(&mut self, index: usize, window: Window) {
        self.desktop_state_dirty = true;
        let id = WindowId(self.next_window_id);
        self.next_window_id += 1;
        self.workspaces[index].windows.push(WorkspaceWindow {
            id,
            stacking_order: id.0,
            window: window.clone(),
            minimized: false,
            initial_focus_pending: true,
            parent: None,
            floating: None,
            stack_weight: 10000,
            floating_override: None,
            stay_on_top: false,
            center_override: None,
            initial_rules_applied: false,
            saved_floating: None,
        });
        self.trim_dynamic_workspaces();
        self.refresh_window_hints(&window);
        tracing::info!(window = id.0, workspace = index + 1, "window opened");
        let index = self.workspace_for_window(&window).unwrap_or(index);
        self.remember_focused_window(&window);
        if index == self.active_workspace {
            self.relayout_active_workspace();
            if window.toplevel().is_some() {
                self.focus_managed_window(&window);
            }
        }
        if window.x11_surface().is_some() {
            self.finish_new_window_focus(&window);
        }
    }

    fn apply_window_rules_on_open(&mut self, window: &Window) {
        let Some(index) = self.workspace_for_window(window) else {
            return;
        };
        let Some((app_id, title)) = window_identity(window) else {
            return;
        };
        let Some(position) = self.workspaces[index]
            .windows
            .iter()
            .position(|entry| entry.window == *window)
        else {
            return;
        };
        if self.workspaces[index].windows[position].initial_rules_applied {
            return;
        }
        self.workspaces[index].windows[position].initial_rules_applied = true;

        let actions = self.config.window_rules.for_window(&app_id, &title);
        {
            let entry = &mut self.workspaces[index].windows[position];
            if let Some(placement) = actions.placement {
                entry.floating_override =
                    Some(placement == knave_config::WindowPlacement::Floating);
            }
            if let Some(center) = actions.center {
                entry.center_override = Some(center);
                entry.floating = None;
                if center {
                    entry.floating_override = Some(true);
                }
            }
            if let Some(stay_on_top) = actions.stay_on_top {
                entry.stay_on_top = stay_on_top;
            }
        }

        let target = actions
            .workspace
            .map(|workspace| usize::from(workspace - 1))
            .unwrap_or(index);
        if target != index {
            let Some(successor) = target.checked_add(1) else {
                return;
            };
            if !self.ensure_workspace_exists(successor) {
                tracing::warn!(
                    app_id,
                    title,
                    workspace = target + 1,
                    "window rule target workspace could not be materialized"
                );
                return;
            }
            let entry = self.workspaces[index].windows.remove(position);
            let window_id = entry.id;
            let moved_window = entry.window.clone();
            let focus_history = self.workspaces[index]
                .focus_history
                .extract_if(.., |id| *id == window_id)
                .collect::<Vec<_>>();
            let minimized_history = self.workspaces[index]
                .minimized_history
                .extract_if(.., |candidate| candidate == &moved_window)
                .collect::<Vec<_>>();
            let fullscreen = self.workspaces[index]
                .fullscreen
                .filter(|owner| *owner == window_id);
            let maximized = self.workspaces[index]
                .maximized
                .filter(|owner| *owner == window_id);
            if fullscreen.is_some() {
                self.workspaces[index].fullscreen = None;
            }
            if maximized.is_some() {
                self.workspaces[index].maximized = None;
            }
            Self::set_activated(&moved_window, false);
            for unmanaged in &mut self.unmanaged_x11_windows {
                if unmanaged
                    .parent
                    .as_ref()
                    .is_some_and(|parent| *parent == moved_window)
                {
                    unmanaged.workspace = target;
                }
            }
            let destination = &mut self.workspaces[target];
            destination.windows.push(entry);
            destination.focus_history.extend(focus_history);
            destination.minimized_history.extend(minimized_history);
            if fullscreen.is_some() {
                destination.fullscreen = fullscreen;
            }
            if maximized.is_some() {
                destination.maximized = maximized;
            }
            self.trim_dynamic_workspaces();
            self.configure_workspace(target);
            if index == self.active_workspace || target == self.active_workspace {
                self.relayout_active_workspace();
            }
            tracing::info!(
                app_id,
                title,
                workspace = target + 1,
                "window rule selected workspace"
            );
        }
        self.refresh_window_hints(window);
    }

    /// Native parent hints are only complete at the first commit; X11 supplies
    /// them at map time. Later redraws must never repeat this activation policy.
    pub fn finish_new_window_focus(&mut self, window: &Window) {
        self.apply_window_rules_on_open(window);
        let Some(index) = self.workspace_for_window(window) else {
            return;
        };
        let Some(position) = self.workspaces[index]
            .windows
            .iter()
            .position(|entry| entry.window == *window)
        else {
            return;
        };
        if !self.workspaces[index].windows[position].initial_focus_pending {
            return;
        }
        if index != self.active_workspace {
            if !self.config.switch_to_new_window_workspace {
                self.workspaces[index].windows[position].initial_focus_pending = false;
                return;
            }
            self.switch_workspace(index);
        }
        self.workspaces[index].windows[position].initial_focus_pending = false;
        let workspace = &self.workspaces[index];
        let entry = workspace
            .windows
            .iter()
            .find(|entry| entry.window == *window)
            .unwrap();
        let reveal = !self.workspace_has_fullscreen(index)
            && !entry.minimized
            && !workspace.hidden_by_parent(entry)
            && !self.window_is_visible(index, entry.id)
            && workspace
                .maximized
                .is_some_and(|owner| owner != entry.id && !workspace.descendant_of(entry, owner));
        if reveal {
            self.workspaces[index].maximized = None;
            self.relayout_active_workspace();
            self.request_repaint();
        }
        self.focus_managed_window(window);
    }

    pub fn add_window(&mut self, surface: ToplevelSurface) {
        // Associate a direct child with its launch workspace even after switching.
        let pid = surface
            .wl_surface()
            .client()
            .and_then(|client| client.get_credentials(&self.display_handle).ok())
            .map(|credentials| credentials.pid as u32);
        let index = self.workspace_for_pid(pid);
        self.add_workspace_window(index, Window::new_wayland_window(surface));
    }

    pub fn add_x11_window(&mut self, surface: X11Surface) {
        if self.window_for_x11_surface(&surface).is_some() {
            return;
        }
        let index = self.workspace_for_pid(surface.get_client_pid().ok());
        self.add_workspace_window(index, Window::new_x11_window(surface));
    }
    /// Move the transient family together so parent hints cannot pull dialogs back.
    pub(crate) fn move_focused_to_workspace(&mut self, target: usize) -> bool {
        let Some(focused) = self.focused_window() else {
            return false;
        };
        let source = self.active_workspace;
        if source == target {
            return true;
        }
        self.cancel_swipe(0);
        self.cancel_workspace_motion();
        if !self.ensure_workspace_exists(target) {
            return false;
        }
        self.release_pointer_buttons();
        let workspace = &self.workspaces[source];
        let mut root = self.window_id(&focused).unwrap();
        for _ in 0..workspace.windows.len() {
            let Some(parent) = workspace
                .windows
                .iter()
                .find(|entry| entry.id == root)
                .and_then(|entry| entry.parent)
                .filter(|parent| workspace.windows.iter().any(|entry| entry.id == *parent))
            else {
                break;
            };
            root = parent;
        }
        let ids: Vec<_> = workspace
            .windows
            .iter()
            .filter(|entry| entry.id == root || workspace.descendant_of(entry, root))
            .map(|entry| entry.id)
            .collect();
        let workspace = &mut self.workspaces[source];
        let moved: Vec<_> = workspace
            .windows
            .extract_if(.., |entry| ids.contains(&entry.id))
            .collect();
        let history: Vec<_> = workspace
            .focus_history
            .extract_if(.., |id| ids.contains(id))
            .collect();
        let minimized: Vec<_> = workspace
            .minimized_history
            .extract_if(.., |window| {
                moved.iter().any(|entry| entry.window == *window)
            })
            .collect();
        let fullscreen = workspace.fullscreen.filter(|id| ids.contains(id));
        let maximized = workspace.maximized.filter(|id| ids.contains(id));
        if fullscreen.is_some() {
            workspace.fullscreen = None;
        }
        if maximized.is_some() {
            workspace.maximized = None;
        }
        for entry in &moved {
            Self::set_activated(&entry.window, false);
        }
        for entry in &mut self.unmanaged_x11_windows {
            if entry
                .parent
                .as_ref()
                .is_some_and(|parent| moved.iter().any(|entry| entry.window == *parent))
            {
                entry.workspace = target;
            }
        }
        let destination = &mut self.workspaces[target];
        destination.windows.extend(moved);
        destination.focus_history.extend(history);
        destination.minimized_history.extend(minimized);
        if fullscreen.is_some() {
            destination.fullscreen = fullscreen;
        }
        if maximized.is_some() {
            destination.maximized = maximized;
        }
        self.trim_dynamic_workspaces();
        self.configure_workspace(target);
        self.relayout_active_workspace();
        true
    }

    pub fn switch_workspace(&mut self, index: usize) {
        let direction = if index > self.active_workspace {
            crate::workspace_navigation::Direction::Next
        } else {
            crate::workspace_navigation::Direction::Previous
        };
        self.switch_workspace_direction(index, direction);
    }

    pub(crate) fn switch_workspace_direction(
        &mut self,
        index: usize,
        direction: crate::workspace_navigation::Direction,
    ) {
        if !self.ensure_workspace_exists(index) {
            return;
        }
        let source = self.active_workspace;
        self.switch_workspace_immediate(index);
        if source != index {
            self.start_workspace_motion(source, index, direction);
        }
        self.trim_dynamic_workspaces();
    }

    pub(crate) fn switch_workspace_immediate(&mut self, index: usize) {
        self.cancel_swipe(0);
        if !self.ensure_workspace_exists(index) {
            return;
        }
        self.remember_current_window_focus();
        if !self
            .pointer
            .grab_start_data()
            .and_then(|start| start.focus)
            .is_some_and(|(surface, _)| self.layer_surface_visible(&surface))
        {
            self.release_pointer_buttons();
        }
        let old: Vec<_> = self
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.windows.iter().map(|entry| entry.window.clone()))
            .collect();
        for window in old {
            self.space.unmap_elem(&window);
            Self::set_activated(&window, false);
        }
        if index != self.active_workspace {
            self.last_workspace = Some(self.active_workspace);
        }
        self.active_workspace = index;
        self.relayout_active_workspace();
    }

    pub fn relayout_active_workspace(&mut self) {
        if self.interaction.is_some() {
            self.release_pointer_buttons();
        }
        self.desktop_state_dirty = true;
        // Empty workspaces have no client commits to wake the output after
        // a switch or the last window's destruction.
        self.request_repaint();
        self.map_active_workspace();
        // An implicit button grab must not survive minimizing its window.
        if self
            .pointer
            .current_focus()
            .or_else(|| {
                self.pointer
                    .grab_start_data()
                    .and_then(|start| start.focus.map(|(surface, _)| surface))
            })
            .is_some_and(|surface| {
                let root =
                    std::iter::successors(Some(surface), smithay::wayland::compositor::get_parent)
                        .last()
                        .unwrap();
                !self.layer_surface_visible(&root)
                    && !self
                        .window_for_surface(&root)
                        .is_some_and(|window| self.space.element_location(&window).is_some())
            })
        {
            self.release_pointer_buttons();
        }
        self.refresh_pointer_surface(0);
        self.restore_active_workspace_focus();
    }

    pub(crate) fn map_active_workspace(&mut self) {
        self.normalize_stack_weights();
        self.arrange_layers();
        let old: Vec<_> = self
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.windows.iter().map(|entry| entry.window.clone()))
            .collect();
        for window in old {
            self.space.unmap_elem(&window);
        }

        for (window, geometry, fullscreen, maximized, tiled, visible) in
            self.workspace_layout(self.active_workspace)
        {
            Self::configure_window(
                &window,
                geometry.loc,
                geometry.size,
                fullscreen,
                maximized,
                tiled,
            );
            if visible {
                self.space.map_element(window, geometry.loc, false);
            } else {
                Self::set_activated(&window, false);
            }
        }
        self.refresh_unmanaged_x11_windows();
        self.sync_x11_stacking();
    }

    pub fn close_focused_window(&mut self) -> bool {
        if let Some(window) = self.focused_window() {
            self.apply_window_action(&window, WindowAction::Close);
            true
        } else {
            false
        }
    }

    pub fn minimize_focused_window(&mut self) -> bool {
        if let Some(window) = self.focused_window() {
            self.apply_window_action(&window, WindowAction::Minimize);
            true
        } else {
            false
        }
    }

    pub(crate) fn apply_window_action(&mut self, window: &Window, action: WindowAction) {
        self.desktop_state_dirty = true;
        match action {
            WindowAction::Close => {
                if let Some(surface) = window.toplevel() {
                    surface.send_close();
                } else if let Some(surface) = window.x11_surface()
                    && let Err(error) = surface.close()
                {
                    tracing::warn!(%error, window = surface.window_id(), "could not close X11 window");
                }
            }
            WindowAction::Minimize => {
                if self.workspace_for_window(window) == Some(self.active_workspace) {
                    self.release_pointer_buttons();
                }
                let mut changed_active_workspace = false;
                for (index, workspace) in self.workspaces.iter_mut().enumerate() {
                    let Some(entry) = workspace
                        .windows
                        .iter_mut()
                        .find(|entry| entry.window == *window && !entry.minimized)
                    else {
                        continue;
                    };
                    entry.minimized = true;
                    Self::set_activated(&entry.window, false);
                    let window = entry.window.clone();
                    workspace
                        .minimized_history
                        .retain(|candidate| candidate != &window);
                    workspace.minimized_history.push(window);
                    changed_active_workspace = index == self.active_workspace;
                    break;
                }
                if changed_active_workspace {
                    self.relayout_active_workspace();
                }
            }
        }
    }

    pub fn restore_last_minimized_window(&mut self) -> bool {
        let workspace = &mut self.workspaces[self.active_workspace];
        while let Some(window) = workspace.minimized_history.pop() {
            if let Some(entry) = workspace
                .windows
                .iter_mut()
                .find(|entry| entry.window == window && entry.minimized)
            {
                entry.minimized = false;
                self.relayout_active_workspace();
                return true;
            }
        }
        false
    }

    /// Returns `None` for an unknown ID and `Some(false)` for a minimized one.
    pub fn focus_window(&mut self, id: WindowId) -> Option<bool> {
        let (workspace, minimized, surface) =
            self.workspaces
                .iter()
                .enumerate()
                .find_map(|(workspace, state)| {
                    state
                        .windows
                        .iter()
                        .find(|entry| entry.id == id)
                        .map(|entry| {
                            (
                                workspace,
                                entry.minimized || state.hidden_by_parent(entry),
                                KeyboardFocus::for_window(&entry.window),
                            )
                        })
                })?;
        if minimized {
            return Some(false);
        }
        if workspace != self.active_workspace {
            self.switch_workspace(workspace);
        }
        if !self
            .workspace_layout(workspace)
            .iter()
            .any(|(window, _, _, _, _, visible)| {
                *visible && KeyboardFocus::for_window(window) == surface
            })
        {
            self.workspaces[workspace].fullscreen = None;
            self.workspaces[workspace].maximized = None;
            self.relayout_active_workspace();
        }
        let Some(surface) = surface else {
            return Some(false);
        };
        let window = self.workspaces[self.active_workspace]
            .windows
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.window.clone());
        if let Some(window) = window {
            debug_assert_eq!(KeyboardFocus::for_window(&window), Some(surface));
            self.focus_managed_window(&window);
        }
        Some(true)
    }

    /// Cycle keyboard focus across the active workspace's windows.
    ///
    /// Targets are unminimized windows that are not hidden behind a minimized
    /// ancestor. The walk order follows the configured focus cycle order and
    /// wraps around; symmetric reverse steps keep the cycle symmetric.
    pub fn cycle_focus(&mut self, step: FocusCycleStep) {
        let index = self.active_workspace;
        let mut candidates = self.focus_cycle_candidates(index);
        if candidates.is_empty() {
            return;
        }
        let current = self
            .focused_window()
            .and_then(|focused| candidates.iter().position(|(_, window)| *window == focused));
        let sequence = self.focus_cycle_sequence(index, &candidates);
        let Some(target) = Self::next_focus_target(&sequence, current, step) else {
            return;
        };
        let target_window = candidates.remove(target).1;
        // A cycle step must not rewrite the recency order: the walk recomputes
        // from the current focus each press, so repeated presses keep moving
        // through the whole recency list instead of oscillating.
        self.reveal_and_focus(index, target_window, false);
    }

    /// Windows a focus switch may target on workspace `index`, in layout
    /// insertion order.
    fn focus_cycle_candidates(&self, index: usize) -> Vec<(WindowId, Window)> {
        let workspace = &self.workspaces[index];
        workspace
            .windows
            .iter()
            .filter(|entry| !entry.minimized && !workspace.hidden_by_parent(entry))
            .map(|entry| (entry.id, entry.window.clone()))
            .collect()
    }

    /// Cycle walk order over candidate positions for workspace `index` per
    /// the configured order: the visible layout order, or most recently
    /// focused windows first with never-focused windows trailing.
    fn focus_cycle_sequence(&self, index: usize, candidates: &[(WindowId, Window)]) -> Vec<usize> {
        match self.config.focus_cycle_order {
            knave_config::FocusCycleOrder::Layout => return (0..candidates.len()).collect(),
            knave_config::FocusCycleOrder::Mru => {}
        }
        let mut sequence: Vec<usize> = self.workspaces[index]
            .focus_history
            .iter()
            .rev()
            .filter_map(|id| candidates.iter().position(|(candidate, _)| candidate == id))
            .collect();
        // Windows never focused still appear in the cycle, in layout order.
        for position in 0..candidates.len() {
            if !sequence.contains(&position) {
                sequence.push(position);
            }
        }
        sequence
    }

    /// The candidate position a cycle step moves focus to, wrapping at both
    /// ends. Without a focused candidate the first sequence entry receives
    /// focus; a single-candidate cycle stays a no-op.
    fn next_focus_target(
        sequence: &[usize],
        current: Option<usize>,
        step: FocusCycleStep,
    ) -> Option<usize> {
        if sequence.is_empty() {
            return None;
        }
        let Some(sequence_position) =
            current.and_then(|focused| sequence.iter().position(|entry| *entry == focused))
        else {
            return Some(sequence[0]);
        };
        let offset = match step {
            FocusCycleStep::Next => 1,
            FocusCycleStep::Previous => sequence.len() - 1,
        };
        Some(sequence[(sequence_position + offset) % sequence.len()])
    }

    /// Focus the nearest window spatially in a direction across the active
    /// workspace. A missing neighbor leaves focus unchanged; overshoot is
    /// not wrapped in geometry terms.
    pub fn focus_directional(&mut self, direction: FocusDirection) {
        let index = self.active_workspace;
        let Some(current) = self.focused_window() else {
            return;
        };
        let placements = self.workspace_layout(index);
        let placement_of = |window: &Window| {
            placements
                .iter()
                .find(|(candidate, ..)| candidate == window)
                .map(|(_, rect, ..)| rect)
        };
        let Some(current_rect) = placement_of(&current).cloned() else {
            return;
        };
        let workspace = &self.workspaces[index];
        let candidates: Vec<(Window, (f64, f64))> = workspace
            .windows
            .iter()
            .filter(|entry| {
                !entry.minimized && !workspace.hidden_by_parent(entry) && entry.window != current
            })
            .filter_map(|entry| {
                placement_of(&entry.window).map(|rect| (entry.window.clone(), center_of(rect)))
            })
            .collect();
        let centers: Vec<(f64, f64)> = candidates.iter().map(|(_, center)| *center).collect();
        let Some(target) =
            nearest_center_in_direction(&centers, center_of(&current_rect), direction)
                .and_then(|best| candidates.into_iter().nth(best))
        else {
            return;
        };
        self.reveal_and_focus(index, target.0, true);
    }

    /// Move a floating window by a fixed step or swap a tile with its nearest neighbor.
    pub fn move_focused_directional(&mut self, direction: FocusDirection) {
        let Some(current) = self.focused_window() else {
            return;
        };
        let index = self.active_workspace;
        let placements = self.workspace_layout(index);
        let Some((_, rect, fullscreen, maximized, tiled, visible)) =
            placements.iter().find(|entry| entry.0 == current)
        else {
            return;
        };
        if *fullscreen || *maximized || !*visible {
            return;
        }
        if !*tiled {
            let (dx, dy) = match direction {
                FocusDirection::Left => (-32, 0),
                FocusDirection::Right => (32, 0),
                FocusDirection::Up => (0, -32),
                FocusDirection::Down => (0, 32),
            };
            let (min, max) = Self::window_constraints(&current);
            if let Some(entry) = self.workspaces[index]
                .windows
                .iter_mut()
                .find(|entry| entry.window == current)
                && let Some(mut saved) = entry.floating
            {
                saved = move_floating_geometry(saved, *rect, self.output_size, min, max, (dx, dy));
                entry.floating = Some(saved);
                self.desktop_state_dirty = true;
                self.relayout_active_workspace();
            }
            return;
        }
        let candidates: Vec<_> = self
            .tiled_slots()
            .into_iter()
            .filter(|(window, _)| *window != current)
            .collect();
        let centers: Vec<_> = candidates.iter().map(|(_, rect)| center_of(rect)).collect();
        if let Some(target) = nearest_center_in_direction(&centers, center_of(rect), direction) {
            self.swap_tiles(&current, &candidates[target].0);
            self.relayout_active_workspace();
        }
    }

    /// Focus a window on workspace `index`, leaving fullscreen and maximize
    /// when the target is currently obscured by one of those overlays.
    /// `remember` controls whether the target moves to the front of the
    /// workspace's recency order.
    fn reveal_and_focus(&mut self, index: usize, target: Window, remember: bool) {
        if index != self.active_workspace {
            return;
        }
        let Some(id) = self.window_id(&target) else {
            return;
        };
        if !self.window_is_visible(index, id) {
            self.workspaces[index].fullscreen = None;
            self.workspaces[index].maximized = None;
            self.relayout_active_workspace();
        }
        self.apply_managed_focus(&target, remember);
    }

    pub fn restore_window(&mut self, id: WindowId) -> bool {
        self.desktop_state_dirty = true;
        for (workspace_index, workspace) in self.workspaces.iter_mut().enumerate() {
            let Some(entry) = workspace.windows.iter_mut().find(|entry| entry.id == id) else {
                continue;
            };
            if entry.minimized {
                entry.minimized = false;
                let window = entry.window.clone();
                workspace
                    .minimized_history
                    .retain(|candidate| candidate != &window);
                if workspace_index == self.active_workspace {
                    self.relayout_active_workspace();
                }
            }
            return true;
        }
        false
    }

    pub fn window_info(&self) -> Vec<WindowInfo> {
        let focused = self
            .keyboard
            .current_focus()
            .and_then(|focus| focus.wl_surface().map(|surface| surface.into_owned()));
        self.workspaces
            .iter()
            .enumerate()
            .flat_map(|(workspace, state)| {
                let focused = focused.clone();
                state.windows.iter().map(move |entry| {
                    let (title, app_id, wl_surface) = if let Some(surface) = entry.window.toplevel()
                    {
                        let (title, app_id) = smithay::wayland::compositor::with_states(
                            surface.wl_surface(),
                            |states| {
                                let attributes = states
                                    .data_map
                                    .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                                    .expect("XDG toplevel data")
                                    .lock()
                                    .unwrap();
                                (
                                    attributes.title.clone().unwrap_or_default(),
                                    attributes.app_id.clone().unwrap_or_default(),
                                )
                            },
                        );
                        (title, app_id, Some(surface.wl_surface().clone()))
                    } else if let Some(surface) = entry.window.x11_surface() {
                        (surface.title(), surface.class(), surface.wl_surface())
                    } else {
                        (String::new(), String::new(), None)
                    };
                    WindowInfo {
                        id: entry.id,
                        title,
                        app_id,
                        workspace: WorkspaceId((workspace + 1) as u32),
                        minimized: entry.minimized,
                        floating: entry.floating.is_some(),
                        fullscreen: state.fullscreen == Some(entry.id),
                        maximized: state.maximized == Some(entry.id),
                        focused: focused
                            .as_ref()
                            .is_some_and(|focused| wl_surface.as_ref() == Some(focused)),
                    }
                })
            })
            .collect()
    }

    pub fn workspace_info(&self) -> Vec<WorkspaceInfo> {
        self.workspaces
            .iter()
            .enumerate()
            .map(|(index, workspace)| WorkspaceInfo {
                workspace: WorkspaceId((index + 1) as u32),
                active: index == self.active_workspace,
                window_count: workspace.windows.len() as u32,
                visible_window_count: self
                    .workspace_layout(index)
                    .iter()
                    .filter(|(_, _, _, _, _, visible)| *visible)
                    .count() as u32,
            })
            .collect()
    }

    pub fn active_window_info(&self) -> Option<WindowInfo> {
        self.window_info().into_iter().find(|window| window.focused)
    }

    pub(crate) fn focused_window(&self) -> Option<Window> {
        let focused = self.keyboard.current_focus()?;
        self.workspaces[self.active_workspace]
            .windows
            .iter()
            .find(|entry| KeyboardFocus::for_window(&entry.window).as_ref() == Some(&focused))
            .map(|entry| entry.window.clone())
    }

    fn remember_focused_window(&mut self, window: &Window) {
        for workspace in &mut self.workspaces {
            let Some(id) = workspace
                .windows
                .iter()
                .find(|entry| entry.window == *window)
                .map(|entry| entry.id)
            else {
                continue;
            };
            workspace.focus_history.retain(|candidate| *candidate != id);
            workspace.focus_history.push(id);
            return;
        }
    }

    fn remember_current_window_focus(&mut self) {
        let Some(focus) = self.keyboard.current_focus() else {
            return;
        };
        let window = self
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.windows.iter())
            .find(|entry| KeyboardFocus::for_window(&entry.window).as_ref() == Some(&focus))
            .map(|entry| entry.window.clone());
        if let Some(window) = window {
            self.remember_focused_window(&window);
        }
    }

    pub(crate) fn window_id(&self, window: &Window) -> Option<WindowId> {
        self.workspaces
            .iter()
            .flat_map(|workspace| workspace.windows.iter())
            .find(|entry| entry.window == *window)
            .map(|entry| entry.id)
    }

    fn window_is_visible(&self, index: usize, id: WindowId) -> bool {
        let Some(entry) = self.workspaces[index]
            .windows
            .iter()
            .find(|entry| entry.id == id)
        else {
            return false;
        };
        self.workspace_layout(index)
            .into_iter()
            .any(|(window, _, _, _, _, visible)| visible && window == entry.window)
    }

    fn last_visible_window(&self, index: usize) -> Option<Window> {
        let history = self.workspaces[index].focus_history.iter().rev().copied();
        for id in history {
            if self.window_is_visible(index, id) {
                return self.workspaces[index]
                    .windows
                    .iter()
                    .find(|entry| entry.id == id)
                    .map(|entry| entry.window.clone());
            }
        }
        self.workspaces[index]
            .windows
            .iter()
            .rev()
            .find(|entry| self.window_is_visible(index, entry.id))
            .map(|entry| entry.window.clone())
    }

    fn active_focus_is_valid(&self) -> bool {
        let Some(focus) = self.keyboard.current_focus() else {
            return false;
        };
        if self
            .exclusive_layer_focus()
            .as_ref()
            .is_some_and(|layer| layer == &focus)
        {
            return true;
        }
        self.workspaces[self.active_workspace]
            .windows
            .iter()
            .any(|entry| {
                self.window_is_visible(self.active_workspace, entry.id)
                    && KeyboardFocus::for_window(&entry.window).as_ref() == Some(&focus)
            })
    }

    pub(crate) fn focus_managed_window(&mut self, window: &Window) -> bool {
        self.apply_managed_focus(window, true)
    }

    fn raise_floating_window(&mut self, window: &Window) {
        let workspace = &self.workspaces[self.active_workspace];
        let Some(entry) = workspace
            .windows
            .iter()
            .find(|entry| entry.window == *window && entry.floating.is_some())
        else {
            return;
        };
        let target = entry.id;
        let mut root = target;
        for _ in 0..workspace.windows.len() {
            let Some(parent) = workspace
                .windows
                .iter()
                .find(|entry| entry.id == root)
                .and_then(|entry| entry.parent)
                .and_then(|id| workspace.windows.iter().find(|entry| entry.id == id))
                .filter(|entry| entry.floating.is_some())
            else {
                break;
            };
            root = parent.id;
        }
        // Keep floating families together, with descendants above their parents.
        // This order is independent of tiling and MRU keyboard-cycle order.
        let mut order: Vec<_> = self
            .workspace_layout(self.active_workspace)
            .into_iter()
            .map(|(window, ..)| window)
            .collect();
        order.sort_by_key(|window| {
            let entry = workspace
                .windows
                .iter()
                .find(|entry| entry.window == *window)
                .unwrap();
            (
                entry.id == root || workspace.descendant_of(entry, root),
                entry.id == target || workspace.descendant_of(entry, target),
            )
        });
        let changed = order.iter().enumerate().any(|(rank, window)| {
            workspace
                .windows
                .iter()
                .any(|entry| entry.window == *window && entry.stacking_order != rank as u64)
        });
        if !changed {
            return;
        }
        for (rank, window) in order.iter().enumerate() {
            let entry = self.workspaces[self.active_workspace]
                .windows
                .iter_mut()
                .find(|entry| entry.window == *window)
                .unwrap();
            entry.stacking_order = rank as u64;
        }
        // Reuse layout order for rendering, hit testing, previews and XWayland.
        for (window, _, _, _, tiled, visible) in self.workspace_layout(self.active_workspace) {
            if !tiled && visible {
                self.space.raise_element(&window, false);
            }
        }
        self.refresh_unmanaged_x11_windows();
        self.sync_x11_stacking();
        self.request_repaint();
    }

    fn apply_managed_focus(&mut self, window: &Window, remember: bool) -> bool {
        let Some(index) = self.workspace_for_window(window) else {
            return false;
        };
        let Some(id) = self.window_id(window) else {
            return false;
        };
        if !self.host_focused
            || index != self.active_workspace
            || !self.window_is_visible(index, id)
        {
            return false;
        }
        let Some(surface) = KeyboardFocus::for_window(window) else {
            return false;
        };
        if remember {
            self.remember_focused_window(window);
        }
        for entry in &self.workspaces[self.active_workspace].windows {
            Self::set_activated(&entry.window, entry.window == *window);
        }
        let surface = self.exclusive_layer_focus().unwrap_or(surface);
        if KeyboardFocus::for_window(window).as_ref() == Some(&surface) {
            self.raise_floating_window(window);
        }
        if self.keyboard.current_focus().as_ref() != Some(&surface) {
            self.keyboard
                .clone()
                .set_focus(self, Some(surface), SERIAL_COUNTER.next_serial());
        }
        true
    }

    pub fn restore_active_workspace_focus(&mut self) {
        self.restore_active_workspace_focus_inner(false);
    }

    fn restore_active_workspace_focus_inner(&mut self, force: bool) {
        if !self.host_focused
            || self.keyboard.is_grabbed()
            || (!force && self.active_focus_is_valid())
        {
            return;
        }
        if let Some(window) = self.last_visible_window(self.active_workspace) {
            self.focus_managed_window(&window);
        } else if self.keyboard.current_focus().is_some() {
            self.keyboard
                .clone()
                .set_focus(self, None, SERIAL_COUNTER.next_serial());
        }
    }

    fn window_for_toplevel(&self, surface: &ToplevelSurface) -> Option<Window> {
        self.workspaces
            .iter()
            .flat_map(|workspace| &workspace.windows)
            .find(|entry| entry.window.toplevel() == Some(surface))
            .map(|entry| entry.window.clone())
    }

    pub fn window_for_surface(
        &self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) -> Option<Window> {
        self.workspaces
            .iter()
            .flat_map(|workspace| &workspace.windows)
            .find(|entry| Self::window_surface(&entry.window).as_ref() == Some(surface))
            .map(|entry| entry.window.clone())
            .or_else(|| {
                self.unmanaged_x11_windows
                    .iter()
                    .find(|entry| Self::window_surface(&entry.window).as_ref() == Some(surface))
                    .map(|entry| entry.window.clone())
            })
    }

    pub fn workspace_for_window(&self, window: &Window) -> Option<usize> {
        self.workspaces
            .iter()
            .position(|workspace| {
                workspace
                    .windows
                    .iter()
                    .any(|entry| entry.window == *window)
            })
            .or_else(|| {
                self.unmanaged_x11_windows
                    .iter()
                    .find(|entry| entry.window == *window)
                    .map(|entry| entry.workspace)
            })
    }

    pub fn window_for_x11_surface(&self, surface: &X11Surface) -> Option<Window> {
        self.workspaces
            .iter()
            .flat_map(|workspace| &workspace.windows)
            .map(|entry| &entry.window)
            .chain(self.unmanaged_x11_windows.iter().map(|entry| &entry.window))
            .find(|window| window.x11_surface() == Some(surface))
            .cloned()
    }

    pub fn window_for_x11_id(&self, id: u32) -> Option<Window> {
        self.workspaces
            .iter()
            .flat_map(|workspace| &workspace.windows)
            .map(|entry| &entry.window)
            .chain(self.unmanaged_x11_windows.iter().map(|entry| &entry.window))
            .find(|window| {
                window
                    .x11_surface()
                    .is_some_and(|surface| surface.window_id() == id)
            })
            .cloned()
    }

    pub(crate) fn input_shape(
        &self,
        window: &Window,
        rect: Geometry,
    ) -> (Geometry, [f32; 4], [f32; 4]) {
        let view = self
            .workspaces
            .iter()
            .find_map(|workspace| {
                let entry = workspace
                    .windows
                    .iter()
                    .find(|entry| entry.window == *window)?;
                let appearance = &self.config.appearance;
                Some(if workspace.fullscreen == Some(entry.id) {
                    knave_config::appearance::ViewEffects::NONE
                } else if workspace.maximized == Some(entry.id) {
                    appearance.views.maximized
                } else if entry.floating.is_some() {
                    appearance.views.floating
                } else {
                    appearance.views.tiled
                })
            })
            .unwrap_or(knave_config::appearance::ViewEffects::NONE);
        let width = if view.borders {
            self.config.appearance.border.width
        } else {
            Default::default()
        };
        let outer = Geometry::new(
            (
                rect.loc.x - i32::from(width.left),
                rect.loc.y - i32::from(width.top),
            )
                .into(),
            (
                rect.size.w + i32::from(width.left) + i32::from(width.right),
                rect.size.h + i32::from(width.top) + i32::from(width.bottom),
            )
                .into(),
        );
        let radii = crate::appearance::render::normalized_radii(
            if view.radius {
                self.config.appearance.border.radius
            } else {
                Default::default()
            },
            outer.size,
        );
        let inner = [
            (radii[0] - f32::from(width.top.max(width.left))).max(0.0),
            (radii[1] - f32::from(width.top.max(width.right))).max(0.0),
            (radii[2] - f32::from(width.bottom.max(width.right))).max(0.0),
            (radii[3] - f32::from(width.bottom.max(width.left))).max(0.0),
        ];
        (outer, radii, inner)
    }

    pub(crate) fn input_window_at_pointer(
        &self,
    ) -> Option<(Window, Point<i32, smithay::utils::Logical>)> {
        if self.timed_workspace_motion() {
            return None;
        }
        // Match rendered bounds while a client retains an old or oversized buffer.
        let layout = self.workspace_layout(self.active_workspace);
        for window in self.space.elements().rev() {
            let location = self.space.element_location(window)?;
            let origin = location - window.geometry().loc;
            let Some((surface, _)) = window.surface_under(
                self.pointer_location - origin.to_f64(),
                WindowSurfaceType::ALL,
            ) else {
                continue;
            };
            let rect = layout
                .iter()
                .find(|entry| entry.0 == *window && (entry.2 || entry.3 || entry.4))
                .map(|entry| entry.1)
                .unwrap_or_else(|| Geometry::new(location, window.geometry().size));
            let root =
                std::iter::successors(Some(surface), smithay::wayland::compositor::get_parent)
                    .last();
            if root == Self::window_surface(window) {
                let (_, _, radii) = self.input_shape(window, rect);
                if !crate::appearance::rounded_contains(rect, radii, self.pointer_location) {
                    continue;
                }
            }
            return Some((window.clone(), origin));
        }
        None
    }

    fn border_window_at_pointer(&self) -> Option<(Window, Point<i32, smithay::utils::Logical>)> {
        if self.timed_workspace_motion() {
            return None;
        }
        let layout = self.workspace_layout(self.active_workspace);
        for window in self.space.elements().rev() {
            let location = self.space.element_location(window)?;
            let rect = layout
                .iter()
                .find(|entry| entry.0 == *window && (entry.2 || entry.3 || entry.4))
                .map(|entry| entry.1)
                .unwrap_or_else(|| Geometry::new(location, window.geometry().size));
            let (outer, radii, _) = self.input_shape(window, rect);
            if !rect.to_f64().contains(self.pointer_location)
                && crate::appearance::rounded_contains(outer, radii, self.pointer_location)
            {
                return Some((window.clone(), location - window.geometry().loc));
            }
        }
        None
    }

    fn focus_layer_at_pointer(&mut self) -> bool {
        if let Some(focus) = self.exclusive_layer_focus() {
            self.focus_layer(focus);
            return true;
        }
        if self.host_focused
            && let Some((_, _, Some(focus))) = self.layer_under_pointer(true)
        {
            self.focus_layer(focus);
            return true;
        }
        if self.host_focused
            && self.input_window_at_pointer().is_none()
            && self.border_window_at_pointer().is_none()
            && let Some((_, _, Some(focus))) = self.layer_under_pointer(false)
        {
            self.focus_layer(focus);
            return true;
        }
        false
    }

    pub fn focus_window_at_pointer(&mut self) {
        if self.focus_layer_at_pointer() {
            return;
        }
        if self.host_focused && matches!(self.layer_under_pointer(true), Some((_, _, None))) {
            // A noninteractive panel receives the click, not the obscured window.
            self.restore_active_workspace_focus();
            return;
        }
        let hit = self
            .input_window_at_pointer()
            .or_else(|| self.border_window_at_pointer());
        let keyboard_surface =
            hit.as_ref()
                .filter(|_| self.host_focused)
                .and_then(|(window, _)| {
                    if window
                        .x11_surface()
                        .is_some_and(|surface| surface.is_override_redirect())
                    {
                        self.unmanaged_x11_windows
                            .iter()
                            .find(|entry| entry.window == *window)
                            .and_then(|entry| entry.parent.as_ref())
                            .and_then(KeyboardFocus::for_window)
                            .or_else(|| {
                                self.keyboard.current_focus().filter(|focus| {
                                    self.space.elements().any(|window| {
                                        KeyboardFocus::for_window(window).as_ref() == Some(focus)
                                    })
                                })
                            })
                    } else {
                        KeyboardFocus::for_window(window)
                    }
                });
        if keyboard_surface.is_none() && self.host_focused {
            // Empty space must not deactivate a visible workspace window.
            self.restore_active_workspace_focus();
            return;
        }
        let focused_window = keyboard_surface.as_ref().and_then(|focus| {
            self.workspaces
                .iter()
                .flat_map(|workspace| workspace.windows.iter())
                .find(|entry| KeyboardFocus::for_window(&entry.window).as_ref() == Some(focus))
                .map(|entry| entry.window.clone())
        });
        if let Some(window) = focused_window.as_ref() {
            self.remember_focused_window(window);
            self.raise_floating_window(window);
        }
        if self.keyboard.current_focus() != keyboard_surface {
            for entry in &self.workspaces[self.active_workspace].windows {
                let activated = keyboard_surface.as_ref().is_some_and(|surface| {
                    KeyboardFocus::for_window(&entry.window).as_ref() == Some(surface)
                });
                Self::set_activated(&entry.window, activated);
            }
            let keyboard = self.keyboard.clone();
            keyboard.set_focus(self, keyboard_surface, SERIAL_COUNTER.next_serial());
        }
    }
    /// End any drag before focus moves away from its application.
    pub fn release_pointer_buttons(&mut self) {
        self.cancel_window_interaction();
        let pointer = self.pointer.clone();
        for button in std::mem::take(&mut self.pressed_buttons) {
            pointer.button(
                self,
                &smithay::input::pointer::ButtonEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time: 0,
                    button,
                    state: smithay::backend::input::ButtonState::Released,
                },
            );
        }
        pointer.unset_grab(self, SERIAL_COUNTER.next_serial(), 0);
        pointer.frame(self);
    }
    pub fn refresh_pointer(&mut self, time: u32) {
        self.request_repaint();
        self.refresh_pointer_surface(time);
        if !self.host_focused {
            if self.keyboard.current_focus().is_some() {
                self.keyboard
                    .clone()
                    .set_focus(self, None, SERIAL_COUNTER.next_serial());
            }
        } else if !self.pointer.is_grabbed() {
            self.focus_layer_at_pointer();
        }
    }

    pub fn refresh_pointer_and_focus(&mut self, time: u32) {
        self.request_repaint();
        self.refresh_pointer_surface(time);
        if !self.pointer.is_grabbed() {
            self.focus_window_at_pointer();
            // Raising a floating family can expose a dialog under the cursor.
            self.refresh_pointer_surface(time);
        }
    }

    /// Backend motion entry point: refresh cursor and hit-test visuals, and
    /// move keyboard focus to the window under the pointer when focus follow
    /// is enabled (hover-to-focus; otherwise focus switches on click only).
    pub fn refresh_pointer_motion(&mut self, time: u32) {
        if self.config.input.focus_follows_mouse {
            self.refresh_pointer_and_focus(time);
        } else {
            self.refresh_pointer(time);
        }
    }

    pub fn refresh_pointer_surface(&mut self, time: u32) {
        let hit = self.input_window_at_pointer();
        let focus = if self.host_focused {
            self.layer_under_pointer(true)
                .map(|(surface, origin, _)| (surface, origin))
                .or_else(|| {
                    hit.and_then(|(window, origin)| {
                        window
                            .surface_under(
                                self.pointer_location - origin.to_f64(),
                                WindowSurfaceType::ALL,
                            )
                            .map(|(surface, location)| {
                                (surface, location.to_f64() + origin.to_f64())
                            })
                    })
                })
                .or_else(|| {
                    self.layer_under_pointer(false)
                        .map(|(surface, origin, _)| (surface, origin))
                })
        } else {
            None
        };
        let pointer = self.pointer.clone();
        pointer.motion(
            self,
            focus,
            &MotionEvent {
                location: self.pointer_location,
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
        pointer.frame(self);
        if std::mem::take(&mut self.interaction_cleanup_pending) {
            self.release_pointer_buttons();
        }
        let split_cursor = self.split_drag_active || self.split_under_pointer().is_some();
        self.cursor.set_split_override(split_cursor);
        self.update_interaction_cursor();
    }

    pub fn remove_window(&mut self, surface: &ToplevelSurface) {
        self.desktop_state_dirty = true;
        if let Some(window) = self.window_for_toplevel(surface) {
            self.remove_managed_window(&window);
        }
    }

    pub fn remove_x11_window(&mut self, surface: &X11Surface) {
        self.desktop_state_dirty = true;
        if let Some(window) = self.window_for_x11_surface(surface) {
            if surface.is_override_redirect() {
                if self.pointer.current_focus() == Self::window_surface(&window) {
                    self.release_pointer_buttons();
                }
                self.space.unmap_elem(&window);
                self.unmanaged_x11_windows
                    .retain(|entry| entry.window != window);
                self.trim_dynamic_workspaces();
                self.refresh_unmanaged_x11_windows();
                self.sync_x11_stacking();
                self.refresh_pointer(0);
            } else {
                self.remove_managed_window(&window);
            }
        }
    }

    fn remove_managed_window(&mut self, target: &Window) {
        self.appearance_renderer.clear();
        for workspace in &mut self.workspaces {
            let removed: Vec<_> = workspace
                .windows
                .extract_if(.., |entry| &entry.window == target)
                .map(|entry| (entry.id, entry.window))
                .collect();
            if !workspace
                .windows
                .iter()
                .any(|entry| Some(entry.id) == workspace.fullscreen)
            {
                workspace.fullscreen = None;
            }
            for (id, window) in removed {
                if workspace.maximized == Some(id) {
                    workspace.maximized = None;
                }
                self.space.unmap_elem(&window);
                workspace.focus_history.retain(|candidate| *candidate != id);
                workspace
                    .minimized_history
                    .retain(|candidate| candidate != &window);
            }
        }
        self.trim_dynamic_workspaces();
        self.relayout_active_workspace();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn master_stack_layouts_zero_to_three_windows() {
        let output = (800, 600).into();
        assert!(master_stack_layout(output, 0, 5000).is_empty());
        assert_eq!(
            master_stack_layout(output, 1, 5000),
            vec![((0, 0).into(), (800, 600).into())]
        );
        assert_eq!(
            master_stack_layout(output, 2, 5000),
            vec![
                ((0, 0).into(), (400, 600).into()),
                ((400, 0).into(), (400, 600).into()),
            ]
        );
        assert_eq!(
            master_stack_layout(output, 3, 5000),
            vec![
                ((0, 0).into(), (400, 600).into()),
                ((400, 0).into(), (400, 300).into()),
                ((400, 300).into(), (400, 300).into()),
            ]
        );
    }

    #[test]
    fn ratios_preserve_coverage_and_single_window_ignores_split() {
        for width in [10, 801, 1920] {
            for ratio in [1000, 3333, 6000, 9000] {
                let layout = master_stack_layout((width, 601).into(), 4, ratio);
                assert_eq!(layout[0].1.w + layout[1].1.w, width);
                assert_eq!(layout[1].0.x, layout[0].1.w);
                assert_eq!(layout[1..].iter().map(|(_, size)| size.h).sum::<i32>(), 601);
                assert_eq!(
                    master_stack_layout((width, 601).into(), 1, ratio)[0].1.w,
                    width
                );
            }
        }
    }

    #[test]
    fn stack_absorbs_integer_remainders() {
        let layout = master_stack_layout((801, 601).into(), 4, 5000);
        assert_eq!(layout[0], ((0, 0).into(), (400, 601).into()));
        assert_eq!(layout[3], ((400, 400).into(), (401, 201).into()));
    }

    #[test]
    fn cycle_target_walks_wraps_and_restarts_without_focus() {
        use crate::dispatch::FocusCycleStep;
        let sequence = [3, 1, 0, 2];
        assert_eq!(
            Villain::next_focus_target(&sequence, Some(0), FocusCycleStep::Next),
            Some(2)
        );
        assert_eq!(
            Villain::next_focus_target(&sequence, Some(2), FocusCycleStep::Next),
            Some(3)
        );
        assert_eq!(
            Villain::next_focus_target(&sequence, Some(3), FocusCycleStep::Previous),
            Some(2)
        );
        assert_eq!(
            Villain::next_focus_target(&sequence, Some(0), FocusCycleStep::Previous),
            Some(1)
        );
        // Unfocused or non-candidate focus starts at the sequence head.
        assert_eq!(
            Villain::next_focus_target(&sequence, None, FocusCycleStep::Next),
            Some(3)
        );
        assert_eq!(
            Villain::next_focus_target(&sequence, Some(7), FocusCycleStep::Next),
            Some(3)
        );
        // A single candidate cycle is a no-op in every direction.
        assert_eq!(
            Villain::next_focus_target(&[0], Some(0), FocusCycleStep::Next),
            Some(0)
        );
        assert_eq!(
            Villain::next_focus_target(&[0], Some(0), FocusCycleStep::Previous),
            Some(0)
        );
        assert_eq!(
            Villain::next_focus_target(&[], None, FocusCycleStep::Next),
            None
        );
    }

    #[test]
    fn directional_focus_prefers_nearest_strictly_in_direction() {
        use crate::dispatch::FocusDirection;
        // Three horizontal centers with the origin between the first two.
        let centers = [(100.0, 150.0), (400.0, 150.0), (700.0, 150.0)];
        let origin = (250.0, 150.0);
        assert_eq!(
            nearest_center_in_direction(&centers, origin, FocusDirection::Left),
            Some(0)
        );
        assert_eq!(
            nearest_center_in_direction(&centers, origin, FocusDirection::Right),
            Some(1)
        );
        assert_eq!(
            nearest_center_in_direction(&centers, origin, FocusDirection::Up),
            None
        );
        assert_eq!(
            nearest_center_in_direction(&centers, origin, FocusDirection::Down),
            None
        );
        // Vertical stacking: down without wrap, and distance beats iteration
        // order when two candidates share the origin column.
        let stack = [(200.0, 100.0), (200.0, 500.0), (200.0, 300.0)];
        assert_eq!(
            nearest_center_in_direction(&stack, (200.0, 200.0), FocusDirection::Down),
            Some(2)
        );
        assert_eq!(
            nearest_center_in_direction(&stack, (200.0, 600.0), FocusDirection::Down),
            None,
            "no wrap-off-screen candidates"
        );
        // Same-distance candidates keep the iteration order.
        let tie = [(100.0, 200.0), (500.0, 200.0)];
        assert_eq!(
            nearest_center_in_direction(&tie, (300.0, 200.0), FocusDirection::Right),
            Some(1)
        );
        assert_eq!(
            nearest_center_in_direction(&tie, (0.0, 200.0), FocusDirection::Right),
            Some(0)
        );
    }
}

#[cfg(test)]
#[path = "window_tests.rs"]
mod request_tests;

pub(crate) fn stack_slot_heights(state: &Villain, weights: &[(WindowId, u32)]) -> Vec<i32> {
    let height = state.split_context().map(|c| c.area.size.h).unwrap_or(1);
    weighted_heights(height, &weights.iter().map(|(_, w)| *w).collect::<Vec<_>>())
}
