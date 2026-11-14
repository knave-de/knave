//! Bounded, nonblocking clipboard transfers owned by the surface runtime.
use super::Runtime;
use knave_ui::toolkit::{ElementId, Input, MAX_TEXT_BYTES};
use smithay_client_toolkit::{
    data_device_manager::{
        DataDeviceManagerState, WritePipe,
        data_device::{DataDevice, DataDeviceHandler},
        data_offer::{DataOfferHandler, DragOffer},
        data_source::{CopyPasteSource, DataSourceHandler},
    },
    reexports::calloop::{
        PostAction, RegistrationToken,
        timer::{TimeoutAction, Timer},
    },
};
use std::{
    io::{Read, Write},
    sync::Arc,
    time::Duration,
};
use wayland_client::{
    Connection, QueueHandle,
    protocol::{
        wl_data_device::WlDataDevice, wl_data_device_manager::DndAction,
        wl_data_source::WlDataSource, wl_surface::WlSurface,
    },
};

const MIMES: [&str; 2] = ["text/plain;charset=utf-8", "text/plain"];
struct Transfer {
    id: u64,
    io: RegistrationToken,
    timeout: RegistrationToken,
}
pub(super) struct Clipboard {
    pub manager: Option<DataDeviceManagerState>,
    pub device: Option<DataDevice>,
    source: Option<CopyPasteSource>,
    contents: Arc<[u8]>,
    transfers: Vec<Transfer>,
    next: u64,
    pub serial: Option<u32>,
}
impl Clipboard {
    pub fn clear_seat(&mut self) {
        self.device = None;
        if let Some(source) = self.source.take() {
            source.inner().destroy();
        }
        self.contents = Arc::from([]);
        self.serial = None;
    }
    pub fn new(manager: Option<DataDeviceManagerState>) -> Self {
        Self {
            manager,
            device: None,
            source: None,
            contents: Arc::from([]),
            transfers: Vec::new(),
            next: 0,
            serial: None,
        }
    }
}
impl Runtime {
    pub(super) fn copy_text(&mut self, text: String, qh: &QueueHandle<Self>) {
        if text.len() > MAX_TEXT_BYTES {
            self.clipboard_error("copy exceeds text limit");
            return;
        }
        let (Some(manager), Some(device), Some(serial)) = (
            &self.clipboard.manager,
            &self.clipboard.device,
            self.clipboard.serial,
        ) else {
            self.clipboard_error("clipboard is unavailable without a seat and input serial");
            return;
        };
        let source = manager.create_copy_paste_source(qh, MIMES);
        source.set_selection(device, serial);
        if let Some(previous) = self.clipboard.source.replace(source) {
            previous.inner().destroy();
        }
        self.clipboard.contents = Arc::from(text.into_bytes());
    }
    fn clipboard_error(&mut self, message: &str) {
        if let Some(app) = &mut self.app {
            notify_error(
                app.as_mut(),
                message,
                &mut self.scene_dirty,
                &mut self.exit,
                &self.wake_sender,
            );
        }
        eprintln!("knave-shell: {message}");
    }
    fn finish_transfer(&mut self, id: u64) {
        if let Some(index) = self.clipboard.transfers.iter().position(|t| t.id == id) {
            let transfer = self.clipboard.transfers.swap_remove(index);
            self.loop_handle.remove(transfer.timeout);
        }
    }
    fn register_transfer(&mut self, id: u64, io: RegistrationToken) {
        match self.loop_handle.insert_source(
            Timer::from_duration(Duration::from_secs(2)),
            move |_, _, state| {
                if let Some(index) = state.clipboard.transfers.iter().position(|t| t.id == id) {
                    let transfer = state.clipboard.transfers.swap_remove(index);
                    state.loop_handle.remove(transfer.io);
                    state.clipboard_error("clipboard transfer timed out");
                }
                TimeoutAction::Drop
            },
        ) {
            Ok(timeout) => self.clipboard.transfers.push(Transfer { id, io, timeout }),
            Err(error) => {
                self.loop_handle.remove(io);
                self.clipboard_error(&format!("clipboard timeout registration failed: {error}"));
            }
        }
    }
    pub(super) fn request_paste(&mut self, target: ElementId, qh: &QueueHandle<Self>) {
        if self.clipboard.transfers.len() >= 4 {
            self.clipboard_error("clipboard transfer capacity reached");
            return;
        }
        let Some(offer) = self
            .clipboard
            .device
            .as_ref()
            .and_then(|d| d.data().selection_offer())
        else {
            self.clipboard_error("no clipboard text selection is available");
            return;
        };
        let mime = offer.with_mime_types(|types| {
            MIMES
                .iter()
                .find(|mime| types.iter().any(|t| t == **mime))
                .map(|s| (*s).to_owned())
        });
        let Some(mime) = mime else {
            self.clipboard_error("clipboard has no supported text type");
            return;
        };
        let pipe = match offer.receive(mime) {
            Ok(pipe) => pipe,
            Err(error) => {
                self.clipboard_error(&format!("clipboard receive failed: {error}"));
                return;
            }
        };
        if let Err(error) = nonblocking(&pipe) {
            self.clipboard_error(&format!("clipboard pipe: {error}"));
            return;
        }
        self.clipboard.next = self.clipboard.next.wrapping_add(1);
        let id = self.clipboard.next;
        let mut bytes = Vec::new();
        let qh = qh.clone();
        match self.loop_handle.insert_source(pipe, move |_, file, state| {
            let mut chunk = [0u8; 4096];
            let mut reader: &std::fs::File = file;
            match reader.read(&mut chunk) {
                Ok(0) => {
                    match String::from_utf8(std::mem::take(&mut bytes)) {
                        Ok(text) => {
                            state.app_input(Input::PasteTo { target, text });
                            state.request_draw(&qh);
                        }
                        Err(_) => state.clipboard_error("clipboard text is not valid UTF-8"),
                    }
                    state.finish_transfer(id);
                    PostAction::Remove
                }
                Ok(len) => {
                    if bytes.len() + len > MAX_TEXT_BYTES {
                        state.clipboard_error("clipboard exceeds text limit");
                        state.finish_transfer(id);
                        PostAction::Remove
                    } else {
                        bytes.extend_from_slice(&chunk[..len]);
                        PostAction::Continue
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    PostAction::Continue
                }
                Err(error) => {
                    state.clipboard_error(&format!("clipboard read failed: {error}"));
                    state.finish_transfer(id);
                    PostAction::Remove
                }
            }
        }) {
            Ok(token) => self.register_transfer(id, token),
            Err(error) => self.clipboard_error(&format!("clipboard read registration: {error}")),
        }
    }
    pub(super) fn cancel_clipboard(&mut self) {
        for transfer in self.clipboard.transfers.drain(..) {
            self.loop_handle.remove(transfer.io);
            self.loop_handle.remove(transfer.timeout);
        }
    }
}
// Error callbacks can change UI state without another input event to drive the host.
fn notify_error(
    app: &mut dyn super::Application,
    message: &str,
    dirty: &mut bool,
    exit: &mut bool,
    wake: &super::WakeSender,
) {
    app.host_error(message);
    *dirty = true;
    *exit |= app.should_close();
    if !*exit {
        // A full one-slot channel already contains the redraw we need.
        let _ = wake.try_send(super::RuntimeWake::Redraw);
    }
}

fn nonblocking(fd: &impl std::os::fd::AsFd) -> std::io::Result<()> {
    let flags = rustix::fs::fcntl_getfl(fd)?;
    rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK)?;
    Ok(())
}
impl DataDeviceHandler for Runtime {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataDevice,
        _: f64,
        _: f64,
        _: &WlSurface,
    ) {
    }
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, _: f64, _: f64) {}
    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
    fn drop_performed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
}
impl DataOfferHandler for Runtime {
    fn source_actions(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }
    fn selected_action(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }
}
impl DataSourceHandler for Runtime {
    fn accept_mime(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataSource,
        _: Option<String>,
    ) {
    }
    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource) {
        if self
            .clipboard
            .source
            .as_ref()
            .is_some_and(|s| s.inner() == source)
        {
            self.clipboard.source = None;
            self.clipboard.contents = Arc::from([]);
        }
        source.destroy();
    }
    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}
    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}
    fn action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: DndAction) {}
    fn send_request(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &WlDataSource,
        mime: String,
        pipe: WritePipe,
    ) {
        if !MIMES.contains(&mime.as_str())
            || !self
                .clipboard
                .source
                .as_ref()
                .is_some_and(|s| s.inner() == source)
        {
            return;
        }
        if self.clipboard.transfers.len() >= 4 {
            self.clipboard_error("clipboard transfer capacity reached");
            return;
        }
        if let Err(error) = nonblocking(&pipe) {
            self.clipboard_error(&format!("clipboard pipe: {error}"));
            return;
        }
        let bytes = Arc::clone(&self.clipboard.contents);
        let mut offset = 0;
        self.clipboard.next = self.clipboard.next.wrapping_add(1);
        let id = self.clipboard.next;
        match self.loop_handle.insert_source(pipe, move |_, file, state| {
            let mut writer: &std::fs::File = file;
            match writer.write(&bytes[offset..]) {
                Ok(0) => {
                    state.finish_transfer(id);
                    PostAction::Remove
                }
                Ok(len) => {
                    offset += len;
                    if offset == bytes.len() {
                        state.finish_transfer(id);
                        PostAction::Remove
                    } else {
                        PostAction::Continue
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    PostAction::Continue
                }
                Err(error) => {
                    state.clipboard_error(&format!("clipboard write failed: {error}"));
                    state.finish_transfer(id);
                    PostAction::Remove
                }
            }
        }) {
            Ok(token) => self.register_transfer(id, token),
            Err(error) => self.clipboard_error(&format!("clipboard write registration: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Application, RuntimeWake};
    use knave_ui::{DisplayList, toolkit::TextMeasurer};
    use smithay_client_toolkit::reexports::calloop::{EventLoop, channel};

    #[derive(Default)]
    struct ErrorApp {
        message: String,
        close_on_error: bool,
        close: bool,
    }
    impl Application for ErrorApp {
        fn input(&mut self, _: Input) {
            panic!("no input required for host errors")
        }
        fn host_error(&mut self, message: &str) {
            self.message = message.into();
            self.close = self.close_on_error;
        }
        fn should_close(&self) -> bool {
            self.close
        }
        fn needs_frame(&self) -> bool {
            false
        }
        fn frame(&mut self, _: [f32; 2], _: &mut dyn TextMeasurer) -> &DisplayList {
            unreachable!()
        }
    }

    #[test]
    fn asynchronous_errors_mark_dirty_and_coalesce_a_wake_without_input() {
        let mut event_loop = EventLoop::<usize>::try_new().unwrap();
        let (sender, receiver) = channel::sync_channel(1);
        event_loop
            .handle()
            .insert_source(receiver, |event, _, count| {
                if matches!(event, channel::Event::Msg(RuntimeWake::Redraw)) {
                    *count += 1;
                }
            })
            .unwrap();
        let mut app = ErrorApp::default();
        let (mut dirty, mut exit, mut wakes) = (false, false, 0);
        for _ in 0..100 {
            notify_error(
                &mut app,
                "clipboard transfer timed out",
                &mut dirty,
                &mut exit,
                &sender,
            );
        }
        assert_eq!(app.message, "clipboard transfer timed out");
        assert!(dirty);
        assert!(!exit);
        event_loop
            .dispatch(Some(Duration::ZERO), &mut wakes)
            .unwrap();
        assert_eq!(wakes, 1);
        // Once drained, the next failure must schedule another frame.
        dirty = false;
        notify_error(
            &mut app,
            "clipboard read failed",
            &mut dirty,
            &mut exit,
            &sender,
        );
        event_loop
            .dispatch(Some(Duration::ZERO), &mut wakes)
            .unwrap();
        assert!(dirty);
        assert_eq!(wakes, 2);
    }

    #[test]
    fn host_errors_honor_close_immediately_and_preserve_shutdown() {
        let (sender, _receiver) = channel::sync_channel(1);
        let mut app = ErrorApp {
            close_on_error: true,
            ..Default::default()
        };
        let (mut dirty, mut exit) = (false, false);
        notify_error(
            &mut app,
            "clipboard read failed",
            &mut dirty,
            &mut exit,
            &sender,
        );
        assert!(app.close);
        assert!(exit);
        app.close_on_error = false;
        notify_error(&mut app, "another failure", &mut dirty, &mut exit, &sender);
        assert!(exit);
    }
}
