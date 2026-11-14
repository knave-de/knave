//! Optional text-input-v3 bridge. Compositor batches apply atomically on `done`.
use super::Runtime;
use knave_ui::toolkit::{Input, TextInputState};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::wp::text_input::zv3::client::{
    zwp_text_input_manager_v3::ZwpTextInputManagerV3,
    zwp_text_input_v3::{self, ZwpTextInputV3},
};

#[derive(Default)]
pub(super) struct Ime {
    pub manager: Option<ZwpTextInputManagerV3>,
    pub proxy: Option<ZwpTextInputV3>,
    serial: u32,
    entered: bool,
    enabled: bool,
    last: Option<TextInputState>,
    preedit: Option<String>,
    commit: Option<String>,
    delete: Option<(u32, u32)>,
}
impl Ime {
    pub(super) fn clear_seat(&mut self) {
        if let Some(proxy) = self.proxy.take() {
            proxy.destroy();
        }
        let manager = self.manager.take();
        *self = Self::new(manager);
    }
    pub(super) fn new(manager: Option<ZwpTextInputManagerV3>) -> Self {
        Self {
            manager,
            ..Default::default()
        }
    }
}
impl Runtime {
    pub(super) fn sync_ime(&mut self) {
        let Some(proxy) = &self.ime.proxy else {
            return;
        };
        let state = if self.ime.entered {
            self.app.as_ref().and_then(|app| app.text_input())
        } else {
            None
        };
        if state == self.ime.last {
            return;
        }
        match &state {
            Some(state) => {
                if self.ime.enabled
                    && self
                        .ime
                        .last
                        .as_ref()
                        .is_some_and(|last| last.id != state.id)
                {
                    proxy.disable();
                    proxy.commit();
                    self.ime.serial = self.ime.serial.wrapping_add(1);
                    self.ime.enabled = false;
                }
                if !self.ime.enabled {
                    proxy.enable();
                    self.ime.enabled = true;
                }
                use knave_ui::toolkit::TextPurpose;
                let (hint, purpose) = match state.purpose {
                    TextPurpose::Password => (
                        zwp_text_input_v3::ContentHint::HiddenText
                            | zwp_text_input_v3::ContentHint::SensitiveData,
                        zwp_text_input_v3::ContentPurpose::Password,
                    ),
                    TextPurpose::Number => (
                        zwp_text_input_v3::ContentHint::None,
                        zwp_text_input_v3::ContentPurpose::Number,
                    ),
                    _ => (
                        zwp_text_input_v3::ContentHint::Completion,
                        zwp_text_input_v3::ContentPurpose::Normal,
                    ),
                };
                proxy.set_content_type(hint, purpose);
                let (text, cursor, anchor) = surrounding(state);
                proxy.set_surrounding_text(text, cursor as i32, anchor as i32);
                let r = state.cursor_rectangle;
                proxy.set_cursor_rectangle(
                    r.x as i32,
                    r.y as i32,
                    r.width.max(1.0) as i32,
                    r.height.max(1.0) as i32,
                );
                proxy.commit();
                self.ime.serial = self.ime.serial.wrapping_add(1);
            }
            None => {
                if self.ime.enabled {
                    proxy.disable();
                    proxy.commit();
                    self.ime.serial = self.ime.serial.wrapping_add(1);
                    self.ime.enabled = false;
                }
            }
        }
        self.ime.last = state;
    }
}
fn surrounding(state: &TextInputState) -> (String, usize, usize) {
    let text = &state.text;
    let cursor = state.cursor.min(text.len());
    let mut start = cursor.saturating_sub(2000);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let mut end = (start + 4000).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (
        text[start..end].to_owned(),
        cursor - start,
        state.anchor.clamp(start, end) - start,
    )
}
impl Dispatch<ZwpTextInputManagerV3, ()> for Runtime {
    fn event(
        _: &mut Self,
        _: &ZwpTextInputManagerV3,
        _: wayland_protocols::wp::text_input::zv3::client::zwp_text_input_manager_v3::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<ZwpTextInputV3, ()> for Runtime {
    fn event(
        state: &mut Self,
        _: &ZwpTextInputV3,
        event: zwp_text_input_v3::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            zwp_text_input_v3::Event::Enter { .. } => {
                state.ime.entered = true;
                state.sync_ime();
            }
            zwp_text_input_v3::Event::Leave { .. } => {
                state.ime.entered = false;
                state.ime.preedit = None;
                state.ime.commit = None;
                state.ime.delete = None;
                state.app_input(Input::Preedit(String::new()));
                state.sync_ime();
                state.request_draw(qh);
            }
            zwp_text_input_v3::Event::PreeditString { text, .. } => {
                state.ime.preedit = Some(text.unwrap_or_default())
            }
            zwp_text_input_v3::Event::CommitString { text } => state.ime.commit = text,
            zwp_text_input_v3::Event::DeleteSurroundingText {
                before_length,
                after_length,
            } => state.ime.delete = Some((before_length, after_length)),
            zwp_text_input_v3::Event::Done { serial } => {
                let input = Input::Ime {
                    delete: state.ime.delete.take(),
                    commit: state.ime.commit.take(),
                    preedit: state.ime.preedit.take(),
                };
                if serial == state.ime.serial && state.ime.enabled {
                    state.app_input(input);
                }
                state.sync_ime();
                state.request_draw(qh);
            }
            _ => {}
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn surrounding_window_is_bounded_and_preserves_unicode_boundaries() {
        let text = "नेपाली".repeat(1000);
        let state = TextInputState {
            purpose: knave_ui::toolkit::TextPurpose::Normal,
            id: knave_ui::toolkit::ElementId(1),
            cursor: text.len(),
            anchor: 0,
            text,
            cursor_rectangle: knave_ui::Rect::default(),
        };
        let (text, cursor, anchor) = surrounding(&state);
        assert!(text.len() <= 4000);
        assert_eq!(cursor, text.len());
        assert_eq!(anchor, 0);
    }
}
