use super::Runtime;
use knave_ui::toolkit::CursorShape;
use smithay_client_toolkit::seat::pointer::{CursorIcon, PointerEventKind};

#[derive(Default)]
pub(super) struct CursorState {
    inside: bool,
    position: [f32; 2],
    attempted: Option<CursorShape>,
}
impl CursorState {
    pub fn pointer_event(&mut self, kind: &PointerEventKind, position: [f32; 2]) {
        self.position = position;
        match kind {
            PointerEventKind::Enter { .. } => {
                self.inside = true;
                self.invalidate(); // A new enter serial must establish even the same cursor again.
            }
            PointerEventKind::Leave { .. } => {
                self.inside = false;
                self.invalidate();
            }
            _ => {}
        }
    }
    pub fn invalidate(&mut self) {
        self.attempted = None;
    }
    fn request(&mut self, shape: CursorShape) -> bool {
        if !self.inside || self.attempted == Some(shape) {
            return false;
        }
        // Failed theme lookups retry on re-entry/scale change, not every pointer motion.
        self.attempted = Some(shape);
        true
    }
}

impl Runtime {
    pub(super) fn sync_cursor(&mut self) {
        let shape = self.app.as_ref().map_or_else(
            || {
                if self
                    .scene
                    .hit_test(self.cursor.position[0], self.cursor.position[1])
                    .is_some()
                {
                    CursorShape::Pointer
                } else {
                    CursorShape::Default
                }
            },
            |app| app.cursor(),
        );
        if !self.cursor.request(shape) {
            return;
        }
        let Some(pointer) = &self.pointer else {
            return;
        };
        let result = match icon(shape) {
            Some(icon) => pointer.set_cursor(&self.connection, icon),
            None => pointer.hide_cursor(),
        };
        if let Err(error) = result {
            let message = format!("could not set {shape:?} cursor: {error}");
            eprintln!("knave-shell: {message}");
            if let Some(app) = &mut self.app {
                app.host_error(&message);
            }
            if shape != CursorShape::Default
                && shape != CursorShape::Hidden
                && let Err(error) = pointer.set_cursor(&self.connection, CursorIcon::Default)
            {
                eprintln!("knave-shell: default cursor fallback failed: {error}");
            }
        }
    }
}
fn icon(shape: CursorShape) -> Option<CursorIcon> {
    Some(match shape {
        CursorShape::Default => CursorIcon::Default,
        CursorShape::Pointer => CursorIcon::Pointer,
        CursorShape::Text => CursorIcon::Text,
        CursorShape::Grab => CursorIcon::Grab,
        CursorShape::Grabbing => CursorIcon::Grabbing,
        CursorShape::Move => CursorIcon::Move,
        CursorShape::EwResize => CursorIcon::EwResize,
        CursorShape::NsResize => CursorIcon::NsResize,
        CursorShape::NwseResize => CursorIcon::NwseResize,
        CursorShape::NeswResize => CursorIcon::NeswResize,
        CursorShape::Crosshair => CursorIcon::Crosshair,
        CursorShape::Wait => CursorIcon::Wait,
        CursorShape::Progress => CursorIcon::Progress,
        CursorShape::NotAllowed => CursorIcon::NotAllowed,
        CursorShape::Hidden => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursor_requests_require_entry_and_are_coalesced_until_reentry_or_scale() {
        let mut state = CursorState::default();
        assert!(!state.request(CursorShape::Default));
        state.pointer_event(&PointerEventKind::Enter { serial: 12 }, [10.0, 10.0]);
        assert!(state.request(CursorShape::Default));
        for i in 0..10000 {
            state.pointer_event(&PointerEventKind::Motion { time: i }, [i as f32, 10.0]);
            assert!(!state.request(CursorShape::Default));
        }
        assert!(state.request(CursorShape::Text));
        assert!(!state.request(CursorShape::Text));
        state.invalidate();
        assert!(state.request(CursorShape::Text));
        state.pointer_event(&PointerEventKind::Leave { serial: 13 }, [10.0, 10.0]);
        assert!(!state.request(CursorShape::Text));
        state.pointer_event(&PointerEventKind::Enter { serial: 14 }, [10.0, 10.0]);
        assert!(state.request(CursorShape::Text));
    }
    #[test]
    fn shapes_keep_semantics_and_only_explicit_hidden_removes_cursor() {
        assert_eq!(icon(CursorShape::Default), Some(CursorIcon::Default));
        assert_eq!(icon(CursorShape::Text), Some(CursorIcon::Text));
        assert_eq!(icon(CursorShape::Grabbing), Some(CursorIcon::Grabbing));
        assert_eq!(icon(CursorShape::NsResize), Some(CursorIcon::NsResize));
        assert_eq!(icon(CursorShape::Hidden), None);
    }
}
