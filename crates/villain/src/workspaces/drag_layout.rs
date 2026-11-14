//! Layout mutations shared by desktop dragging and future placement callers.
use super::*;

pub(super) fn weighted_heights(height: i32, weights: &[u32]) -> Vec<i32> {
    let total: u64 = weights.iter().map(|w| u64::from((*w).max(1))).sum();
    let mut used = 0;
    let mut cumulative = 0_u64;
    weights
        .iter()
        .map(|weight| {
            cumulative += u64::from((*weight).max(1));
            let end = (i64::from(height) * cumulative as i64 / total as i64) as i32;
            let size = end - used;
            used = end;
            size
        })
        .collect()
}

impl Villain {
    pub(super) fn normalize_stack_weights(&mut self) {
        let weights = self.stack_weights();
        let sum: u64 = weights.iter().map(|(_, weight)| u64::from(*weight)).sum();
        let target = weights.len() as u64 * 10000;
        if sum == 0 || sum == target {
            return;
        }
        for entry in &mut self.workspaces[self.active_workspace].windows {
            if let Some((_, weight)) = weights.iter().find(|(id, _)| *id == entry.id) {
                entry.stack_weight =
                    (u64::from(*weight) * target / sum).clamp(1, u64::from(u32::MAX)) as u32;
            }
        }
    }

    pub(crate) fn toggle_floating_focused(&mut self) -> Result<(), crate::dispatch::DispatchError> {
        use crate::dispatch::DispatchError;
        let window = self
            .focused_window()
            .ok_or(DispatchError::NoFocusedWindow)?;
        let placement = self
            .workspace_layout(self.active_workspace)
            .into_iter()
            .find(|entry| entry.0 == window && entry.5)
            .ok_or(DispatchError::Interaction("window is not visible"))?;
        if placement.2 || placement.3 {
            return Err(DispatchError::Interaction(
                "restore maximized/fullscreen window before changing floating mode",
            ));
        }
        self.release_pointer_buttons();
        let area = self.usable_area();
        let (min, max) = Self::window_constraints(&window);
        let entry = self.workspaces[self.active_workspace]
            .windows
            .iter_mut()
            .find(|entry| entry.window == window)
            .unwrap();
        if let Some(rect) = entry.floating.take() {
            entry.saved_floating = Some(rect);
            entry.floating_override = Some(false);
        } else {
            let size = (area.size.w * 2 / 3, area.size.h * 2 / 3).into();
            let default = Rectangle::new(
                area.loc + Point::from((area.size.w / 6, area.size.h / 6)),
                size,
            );
            entry.floating = Some(constrain_geometry(
                entry.saved_floating.unwrap_or(default),
                self.output_size,
                min,
                max,
            ));
            entry.floating_override = Some(true);
        }
        self.relayout_active_workspace();
        self.focus_managed_window(&window);
        Ok(())
    }

    pub(crate) fn tiled_slots(&self) -> Vec<(Window, Geometry)> {
        self.workspace_layout(self.active_workspace)
            .into_iter()
            .filter(|entry| entry.4 && entry.5 && !entry.2 && !entry.3)
            .map(|entry| (entry.0, entry.1))
            .collect()
    }

    pub(crate) fn stack_weights(&self) -> Vec<(WindowId, u32)> {
        let workspace = &self.workspaces[self.active_workspace];
        workspace
            .windows
            .iter()
            .filter(|entry| {
                entry.floating.is_none() && !entry.minimized && !workspace.hidden_by_parent(entry)
            })
            .skip(1)
            .map(|entry| (entry.id, entry.stack_weight))
            .collect()
    }

    pub(crate) fn apply_stack_weights(&mut self, weights: &[(WindowId, u32)]) {
        for entry in &mut self.workspaces[self.active_workspace].windows {
            if let Some((_, weight)) = weights.iter().find(|(id, _)| *id == entry.id) {
                entry.stack_weight = (*weight).max(1);
            }
        }
        self.map_active_workspace();
        self.request_repaint();
    }

    pub(crate) fn swap_tiles(&mut self, source: &Window, destination: &Window) {
        let workspace = &mut self.workspaces[self.active_workspace];
        let a = workspace
            .windows
            .iter()
            .position(|entry| entry.window == *source);
        let b = workspace
            .windows
            .iter()
            .position(|entry| entry.window == *destination);
        if let (Some(a), Some(b)) = (a, b) {
            // Proportions belong to layout slots, while IDs follow their windows.
            let wa = workspace.windows[a].stack_weight;
            let wb = workspace.windows[b].stack_weight;
            workspace.windows.swap(a, b);
            workspace.windows[a].stack_weight = wa;
            workspace.windows[b].stack_weight = wb;
            self.desktop_state_dirty = true;
            self.map_active_workspace();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn moving_constrained_floating_window_preserves_saved_size() {
        let saved = Rectangle::new((0, 0).into(), (1200, 900).into());
        let output = (800, 600).into();
        let unconstrained = (0, 0).into();
        let moved =
            move_floating_geometry(saved, saved, output, unconstrained, unconstrained, (32, 0));
        assert_eq!(moved.size, saved.size);
        assert_eq!(moved.loc, (0, 0).into());
        let smaller = Rectangle::new((10, 20).into(), (400, 300).into());
        let moved = move_floating_geometry(
            smaller,
            smaller,
            output,
            unconstrained,
            unconstrained,
            (32, 0),
        );
        assert_eq!(moved.loc, (42, 20).into());
        assert_eq!(moved.size, smaller.size);
        let visible = Rectangle::new((100, 20).into(), smaller.size);
        let moved = move_floating_geometry(
            smaller,
            visible,
            output,
            unconstrained,
            unconstrained,
            (32, 0),
        );
        assert_eq!(moved.loc, (132, 20).into());
        assert_eq!(moved.size, smaller.size);
        let moved = move_floating_geometry(
            smaller,
            visible,
            output,
            unconstrained,
            unconstrained,
            (0, 32),
        );
        assert_eq!(moved.loc, (10, 52).into());
    }

    #[test]
    fn weighted_stack_fills_height_without_rounding_gaps() {
        assert_eq!(weighted_heights(601, &[1, 1, 1]), vec![200, 200, 201]);
        assert_eq!(weighted_heights(600, &[1, 2, 3]), vec![100, 200, 300]);
        assert!(weighted_heights(600, &[]).is_empty());
        for height in [1, 601, 1080] {
            assert_eq!(
                weighted_heights(height, &[1, 130, 42, 0])
                    .iter()
                    .sum::<i32>(),
                height
            );
        }
    }
}
