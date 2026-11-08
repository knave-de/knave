//! Hit testing for compositor-owned master and stack dividers.
use crate::{state::Villain, window_grab::Operation, workspaces::master_width};
use smithay::backend::input::ButtonState;

impl Villain {
    pub(crate) fn stack_divider_under_pointer(&self) -> Option<Operation> {
        if !self.can_start_interaction() || self.layer_under_pointer(true).is_some() {
            return None;
        }
        if let Some((window, origin)) = self.input_window_at_pointer() {
            if self.floating_geometry(&window).is_some() {
                return None;
            }
            if let Some((surface, _)) = window.surface_under(
                self.pointer_location - origin.to_f64(),
                smithay::desktop::WindowSurfaceType::ALL,
            ) {
                let root =
                    std::iter::successors(Some(surface), smithay::wayland::compositor::get_parent)
                        .last();
                if root != Self::window_surface(&window) {
                    return None;
                }
            }
        }
        let context = self.split_context()?;
        let slots = self.tiled_slots();
        if slots.len() < 3 {
            return None;
        }
        let weights = self.stack_weights();
        for (pair, neighbors) in slots[1..].windows(2).enumerate() {
            let upper = neighbors[0].1;
            let lower = neighbors[1].1;
            let boundary = (upper.loc.y + upper.size.h + lower.loc.y) / 2;
            if self.pointer_location.x >= f64::from(upper.loc.x)
                && self.pointer_location.x < f64::from(upper.loc.x + upper.size.w)
                && (self.pointer_location.y - f64::from(boundary)).abs() <= 5.0
            {
                // Raw slot heights include decorations and gaps.
                let heights = crate::workspaces::stack_slot_heights(self, &weights);
                if heights[pair] + heights[pair + 1] < 2 {
                    return None;
                }
                return Some(Operation::Stack {
                    context,
                    weights,
                    pair,
                    upper: heights[pair],
                    lower: heights[pair + 1],
                });
            }
        }
        None
    }
    pub(crate) fn try_start_split_drag(
        &mut self,
        button: u32,
        state: ButtonState,
        _time: u32,
    ) -> bool {
        if button != 0x110
            || state != ButtonState::Pressed
            || self.pressed_buttons.len() != 1
            || !self.can_start_interaction()
        {
            return false;
        }
        let operation = if let Some(operation) = self.stack_divider_under_pointer() {
            operation
        } else if let Some(context) = self.split_under_pointer() {
            let boundary = context.area.loc.x
                + master_width(
                    context.area.size.w,
                    self.master_ratio(self.active_workspace),
                );
            Operation::Master {
                initial: self.master_override(),
                offset: self.pointer_location.x - f64::from(boundary),
                context,
            }
        } else {
            return false;
        };
        self.start_compositor_interaction(operation, Some(button));
        true
    }
}
