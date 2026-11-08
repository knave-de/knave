//! Shared, bounded target selection for shortcuts and live workspace previews.
use crate::{state::Villain, workspaces::Workspace};

pub(crate) const MAX_WORKSPACES: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Direction {
    Next,
    Previous,
}
impl Direction {
    pub fn incoming_offset(self, width: i32) -> i32 {
        match self {
            Self::Next => width,
            Self::Previous => -width,
        }
    }
}

fn target(
    occupied: &[bool],
    source: usize,
    direction: Direction,
    skip_empty: bool,
) -> Option<usize> {
    let count = occupied.len();
    if source >= count {
        return None;
    }
    match direction {
        Direction::Next => {
            for (index, is_occupied) in occupied.iter().enumerate().skip(source + 1) {
                if !skip_empty || *is_occupied {
                    return Some(index);
                }
            }
            // If nothing is occupied ahead, expose the next blank slot before
            // growing beyond the current tail.
            Some(if source + 1 < count {
                source + 1
            } else {
                count
            })
        }
        Direction::Previous => (0..source)
            .rev()
            .find(|index| !skip_empty || occupied[*index]),
    }
}

impl Villain {
    pub(crate) fn workspace_target(&self, source: usize, direction: Direction) -> Option<usize> {
        let occupied: Vec<_> = self
            .workspaces
            .iter()
            .map(|workspace| !workspace.is_empty())
            .collect();
        target(
            &occupied,
            source,
            direction,
            self.config.workspace.skip_empty,
        )
    }
    pub(crate) fn workspace_occupied(&self, index: usize) -> bool {
        self.workspaces
            .get(index)
            .is_some_and(|workspace| !workspace.is_empty())
    }

    pub(crate) fn ensure_workspace_exists(&mut self, index: usize) -> bool {
        let Some(count) = index.checked_add(1) else {
            return false;
        };
        if count > MAX_WORKSPACES {
            return false;
        }
        if count <= self.workspaces.len() {
            return true;
        }
        if self
            .workspaces
            .try_reserve(count - self.workspaces.len())
            .is_err()
        {
            return false;
        }
        self.workspaces.resize_with(count, Workspace::default);
        true
    }

    /// Dynamic slots are a contiguous tail. Keep every fixed slot, every slot
    /// through the highest used one, and one empty slot after that.
    pub(crate) fn trim_dynamic_workspaces(&mut self) {
        let fixed_count = self.config.workspace.predefined as usize;
        let highest_used = self
            .workspaces
            .iter()
            .enumerate()
            .filter(|(_, workspace)| !workspace.is_empty())
            .map(|(index, _)| index)
            .chain(
                self.unmanaged_x11_windows
                    .iter()
                    .map(|window| window.workspace),
            )
            .chain(self.children.iter().map(|(index, _)| *index))
            .chain(self.workspace_motion.highest_workspace())
            .chain(
                self.gestures
                    .transition
                    .map(|transition| transition.source.max(transition.target)),
            )
            .max();
        let retained = fixed_count
            .saturating_add(1)
            .max(2)
            .max(self.active_workspace.saturating_add(2))
            .max(highest_used.map_or(0, |index| index.saturating_add(2)))
            .min(MAX_WORKSPACES);
        if retained != self.workspaces.len() {
            self.desktop_state_dirty = true;
        }
        if retained < self.workspaces.len() {
            self.workspaces.truncate(retained);
            if self.last_workspace.is_some_and(|index| index >= retained) {
                self.last_workspace = None;
            }
        } else if retained > self.workspaces.len() {
            self.workspaces.resize_with(retained, Workspace::default);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sparse_slots_skip_without_wrapping_and_keep_a_dynamic_tail() {
        let occupied = [
            true, false, false, true, false, false, true, false, false, false,
        ];
        for (source, next, previous) in [(0, 3, None), (3, 6, Some(0)), (6, 7, Some(3))] {
            assert_eq!(target(&occupied, source, Direction::Next, true), Some(next));
            assert_eq!(
                target(&occupied, source, Direction::Previous, true),
                previous
            );
        }
        assert_eq!(target(&occupied, 1, Direction::Next, true), Some(3));
        assert_eq!(target(&occupied, 1, Direction::Previous, true), Some(0));
        assert_eq!(target(&occupied, 9, Direction::Next, true), Some(10));
    }
    #[test]
    fn disabled_policy_visits_empty_slots_and_grows_at_the_end() {
        assert_eq!(target(&[false; 10], 0, Direction::Next, false), Some(1));
        assert_eq!(target(&[false; 10], 0, Direction::Previous, false), None);
        assert_eq!(target(&[true; 10], 9, Direction::Next, true), Some(10));
    }
    #[test]
    fn nowhere_else_to_go_is_a_no_op() {
        for direction in [Direction::Next, Direction::Previous] {
            assert_eq!(
                target(&[false; 10], 0, direction, true),
                if direction == Direction::Next {
                    Some(1)
                } else {
                    None
                }
            );
            assert_eq!(
                target(&[true, false, false], 0, direction, true),
                if direction == Direction::Next {
                    Some(1)
                } else {
                    None
                }
            );
            assert_eq!(
                target(&[true, false, false], 2, direction, true),
                if direction == Direction::Next {
                    Some(3)
                } else {
                    Some(0)
                }
            );
            assert_eq!(
                target(&[true], 0, direction, false),
                if direction == Direction::Next {
                    Some(1)
                } else {
                    None
                }
            );
            assert_eq!(target(&[], 0, direction, true), None);
            assert_eq!(target(&[true], 9, direction, true), None);
        }
    }
    #[test]
    fn slide_side_depends_on_travel_not_workspace_number() {
        assert_eq!(Direction::Next.incoming_offset(1280), 1280);
        assert_eq!(Direction::Previous.incoming_offset(1280), -1280);
    }
}
