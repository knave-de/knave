//! Compositor-native screenshots use a frozen output frame for interactive regions.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    os::fd::{AsRawFd, OwnedFd},
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::element::{Id, memory::MemoryRenderBuffer},
    },
    reexports::calloop::{
        Interest, Mode, PostAction, RegistrationToken,
        generic::Generic,
        timer::{TimeoutAction, Timer},
    },
    utils::{Buffer, Logical, Physical, Point, Rectangle, Transform},
    wayland::selection::{SelectionTarget, data_device::set_data_device_selection},
};

use crate::{
    capture::{self, CapturedFrame},
    screenshot_notification::ScreenshotNotice,
    state::Villain,
};

const IMAGE_MIME: &str = "image/png";
const MAX_TRANSFERS: usize = 4;
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(5);
const BTN_LEFT: u32 = 0x110;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScreenshotMode {
    Region,
    Output,
    Window,
}

#[derive(Clone)]
pub enum SelectionPayload {
    ForwardToXwayland,
    Png(Arc<[u8]>),
}

struct Selection {
    frame: CapturedFrame,
    background: MemoryRenderBuffer,
    destination: Destination,
    start: Option<Point<f64, Logical>>,
    end: Point<f64, Logical>,
    dragging: bool,
    overlay_ids: [Id; 8],
}

#[derive(Clone, Copy)]
enum Destination {
    Save,
    Clipboard,
}

struct Transfer {
    id: u64,
    io: RegistrationToken,
    timeout: RegistrationToken,
}

#[derive(Default)]
pub(crate) struct ScreenshotState {
    selection: Option<Selection>,
    intercepted_buttons: std::collections::HashSet<u32>,
    pinch_gesture_active: bool,
    pinch_gesture_suppressed: bool,
    hold_gesture_active: bool,
    hold_gesture_suppressed: bool,
    transfers: Vec<Transfer>,
    next_transfer_id: u64,
    notifications: crate::screenshot_notification::ScreenshotNotifier,
}

impl ScreenshotState {
    pub(crate) fn selecting(&self) -> bool {
        self.selection.is_some()
    }

    pub(crate) fn intercepting_pointer(&self) -> bool {
        self.selection.is_some() || !self.intercepted_buttons.is_empty()
    }

    pub(crate) fn clear_intercepted_buttons(&mut self) {
        self.intercepted_buttons.clear();
    }

    pub(crate) fn begin_pinch_gesture(&mut self) -> bool {
        if self.pinch_gesture_active {
            return false;
        }
        self.pinch_gesture_active = true;
        self.pinch_gesture_suppressed = self.intercepting_pointer();
        !self.pinch_gesture_suppressed
    }

    pub(crate) fn update_pinch_gesture(&self) -> bool {
        self.pinch_gesture_active && !self.pinch_gesture_suppressed && !self.intercepting_pointer()
    }

    pub(crate) fn end_pinch_gesture(&mut self) -> bool {
        let forward = self.update_pinch_gesture();
        self.pinch_gesture_active = false;
        self.pinch_gesture_suppressed = false;
        forward
    }

    pub(crate) fn begin_hold_gesture(&mut self) -> bool {
        if self.hold_gesture_active {
            return false;
        }
        self.hold_gesture_active = true;
        self.hold_gesture_suppressed = self.intercepting_pointer();
        !self.hold_gesture_suppressed
    }

    pub(crate) fn update_hold_gesture(&self) -> bool {
        self.hold_gesture_active && !self.hold_gesture_suppressed && !self.intercepting_pointer()
    }

    pub(crate) fn end_hold_gesture(&mut self) -> bool {
        let forward = self.update_hold_gesture();
        self.hold_gesture_active = false;
        self.hold_gesture_suppressed = false;
        forward
    }

    pub(crate) fn clear_gesture_streams(&mut self) {
        self.pinch_gesture_active = false;
        self.pinch_gesture_suppressed = false;
        self.hold_gesture_active = false;
        self.hold_gesture_suppressed = false;
    }

    fn has_forwarded_gesture(&self) -> bool {
        (self.pinch_gesture_active && !self.pinch_gesture_suppressed)
            || (self.hold_gesture_active && !self.hold_gesture_suppressed)
    }

    pub(crate) fn background(&self) -> Option<&MemoryRenderBuffer> {
        self.selection
            .as_ref()
            .map(|selection| &selection.background)
    }

    pub(crate) fn frame_size(&self) -> Option<smithay::utils::Size<i32, Buffer>> {
        self.selection
            .as_ref()
            .map(|selection| selection.frame.size)
    }

    pub(crate) fn selection_rect(&self) -> Option<Rectangle<i32, Physical>> {
        self.selection.as_ref().and_then(selection_rect)
    }

    pub(crate) fn overlay_ids(&self) -> Option<&[Id; 8]> {
        self.selection
            .as_ref()
            .map(|selection| &selection.overlay_ids)
    }

    fn finish_transfer(
        &mut self,
        id: u64,
        handle: &smithay::reexports::calloop::LoopHandle<'static, Villain>,
    ) {
        if let Some(index) = self.transfers.iter().position(|transfer| transfer.id == id) {
            let transfer = self.transfers.swap_remove(index);
            handle.remove(transfer.timeout);
        }
    }
}

pub(crate) fn begin(
    state: &mut Villain,
    mode: ScreenshotMode,
    clipboard: bool,
) -> Result<(), String> {
    if state.screenshot.selecting() || !state.screenshot.intercepted_buttons.is_empty() {
        return Err("a screenshot selection is already active".into());
    }
    if !state.pressed_buttons.is_empty() {
        return Err("finish the current pointer gesture before taking a screenshot".into());
    }
    if state.screenshot.has_forwarded_gesture() {
        return Err("finish the current pointer gesture before taking a screenshot".into());
    }

    let destination = if clipboard {
        Destination::Clipboard
    } else {
        Destination::Save
    };
    state.cancel_swipe(0);
    state.cancel_workspace_motion();
    let frame = capture::screenshot_frame(state)?;
    match mode {
        ScreenshotMode::Region => {
            let background = MemoryRenderBuffer::from_slice(
                &frame.pixels,
                Fourcc::Abgr8888,
                (frame.size.w, frame.size.h),
                1,
                Transform::Normal,
                None,
            );
            let pointer = state.pointer_location;
            state.screenshot.selection = Some(Selection {
                frame,
                background,
                destination,
                start: None,
                end: pointer,
                dragging: false,
                overlay_ids: std::array::from_fn(|_| Id::new()),
            });
            state.cursor.set_screenshot_override(true);
            state.request_repaint();
            tracing::info!("screenshot region selection started");
            Ok(())
        }
        ScreenshotMode::Output => finish(state, frame, None, destination),
        ScreenshotMode::Window => {
            let rect = focused_window_rect(state, frame.scale)?;
            finish(state, frame, Some(rect), destination)
        }
    }
}

pub(crate) fn cancel(state: &mut Villain, restore_pointer_focus: bool) {
    if state.screenshot.selection.take().is_none() {
        return;
    }
    state.cursor.set_screenshot_override(false);
    state.request_repaint();
    if restore_pointer_focus && state.screenshot.intercepted_buttons.is_empty() {
        state.refresh_pointer(0);
    }
    tracing::info!("screenshot region selection cancelled");
}

pub(crate) fn pointer_motion(state: &mut Villain) -> bool {
    let selecting = state.screenshot.selection.is_some();
    if let Some(selection) = state.screenshot.selection.as_mut() {
        if selection.dragging {
            selection.end = state.pointer_location;
        }
        state.request_repaint();
    }
    selecting || !state.screenshot.intercepted_buttons.is_empty()
}

pub(crate) fn pointer_button(state: &mut Villain, button: u32, pressed: bool, time: u32) -> bool {
    let selecting = state.screenshot.selection.is_some();
    if !selecting && !state.screenshot.intercepted_buttons.contains(&button) {
        return false;
    }

    if pressed {
        state.screenshot.intercepted_buttons.insert(button);
        if button == BTN_LEFT
            && let Some(selection) = state.screenshot.selection.as_mut()
        {
            selection.start = Some(state.pointer_location);
            selection.end = state.pointer_location;
            selection.dragging = true;
            state.request_repaint();
        }
        return true;
    }

    let tracked = state.screenshot.intercepted_buttons.remove(&button);
    if button == BTN_LEFT && tracked && selecting {
        let selection = state
            .screenshot
            .selection
            .take()
            .expect("selection checked");
        state.cursor.set_screenshot_override(false);
        if let Some(rect) = selection_rect(&selection) {
            let destination = selection.destination;
            let frame = selection.frame;
            if let Err(error) = finish(state, frame, Some(buffer_rect(rect)), destination) {
                tracing::warn!(%error, "region screenshot failed");
            }
        } else {
            tracing::debug!("screenshot region was empty");
        }
        state.request_repaint();
        if state.screenshot.intercepted_buttons.is_empty() {
            state.refresh_pointer(time);
        }
    } else if !state.screenshot.selection.is_some()
        && state.screenshot.intercepted_buttons.is_empty()
    {
        state.refresh_pointer(time);
    }
    true
}

pub(crate) fn keyboard_key(state: &mut Villain, key: u32, pressed: bool) -> bool {
    if !state.screenshot.selecting() {
        return false;
    }
    if pressed && key == smithay::input::keyboard::keysyms::KEY_Escape {
        cancel(state, true);
    }
    true
}

fn selection_rect(selection: &Selection) -> Option<Rectangle<i32, Physical>> {
    if !selection.dragging {
        return None;
    }
    let start = selection.start?;
    let scale = selection.frame.scale;
    let x0 = (start.x.min(selection.end.x) * scale).floor() as i32;
    let y0 = (start.y.min(selection.end.y) * scale).floor() as i32;
    let x1 = (start.x.max(selection.end.x) * scale).ceil() as i32;
    let y1 = (start.y.max(selection.end.y) * scale).ceil() as i32;
    let x0 = x0.clamp(0, selection.frame.size.w);
    let y0 = y0.clamp(0, selection.frame.size.h);
    let x1 = x1.clamp(0, selection.frame.size.w);
    let y1 = y1.clamp(0, selection.frame.size.h);
    (x1 > x0 && y1 > y0).then(|| Rectangle::new((x0, y0).into(), (x1 - x0, y1 - y0).into()))
}

fn buffer_rect(rect: Rectangle<i32, Physical>) -> Rectangle<i32, Buffer> {
    Rectangle::new(
        (rect.loc.x, rect.loc.y).into(),
        (rect.size.w, rect.size.h).into(),
    )
}

fn focused_window_rect(state: &Villain, scale: f64) -> Result<Rectangle<i32, Buffer>, String> {
    let window = state
        .focused_window()
        .ok_or_else(|| "there is no focused window to capture".to_owned())?;
    let placement = state
        .workspace_layout(state.active_workspace)
        .into_iter()
        .find(|entry| entry.0 == window && entry.5)
        .ok_or_else(|| "the focused window is not visible".to_owned())?;
    let physical = placement.1.to_physical_precise_round(scale);
    Ok(Rectangle::new(
        (physical.loc.x, physical.loc.y).into(),
        (physical.size.w, physical.size.h).into(),
    ))
}

fn finish(
    state: &mut Villain,
    frame: CapturedFrame,
    crop: Option<Rectangle<i32, Buffer>>,
    destination: Destination,
) -> Result<(), String> {
    let (size, pixels) = crop_pixels(&frame, crop)?;
    let encoded = encode_png(size.w as u32, size.h as u32, &pixels)?;
    match destination {
        Destination::Save => {
            let path = save_png(&encoded)?;
            tracing::info!(path = %path.display(), width = size.w, height = size.h, "screenshot saved");
            state
                .screenshot
                .notifications
                .submit(ScreenshotNotice::saved(
                    path,
                    size.w as u32,
                    size.h as u32,
                    &pixels,
                    state.application_environment(),
                ));
        }
        Destination::Clipboard => {
            let payload = SelectionPayload::Png(Arc::from(encoded));
            set_data_device_selection::<Villain>(
                &state.display_handle,
                &state.seat,
                vec![IMAGE_MIME.into()],
                payload,
            );
            if let Err(error) = announce_screenshot_clipboard_to_xwayland(state) {
                tracing::warn!(%error, "could not export screenshot clipboard to XWayland");
            }
            tracing::info!(
                width = size.w,
                height = size.h,
                "screenshot copied to clipboard"
            );
            state
                .screenshot
                .notifications
                .submit(ScreenshotNotice::copied(
                    size.w as u32,
                    size.h as u32,
                    &pixels,
                ));
        }
    }
    Ok(())
}

pub(crate) fn announce_screenshot_clipboard_to_xwayland(state: &mut Villain) -> Result<(), String> {
    let Some(xwm) = state.xwm.as_mut() else {
        return Ok(());
    };
    xwm.new_selection(
        SelectionTarget::Clipboard,
        Some(vec![IMAGE_MIME.to_owned()]),
    )
    .map_err(|error| error.to_string())?;
    // Do not wait for an unrelated X11 reply on the compositor event loop.
    // Smithay queues the owner change and its XWayland source flushes queued
    // requests as part of the normal event-loop cycle.
    Ok(())
}

fn crop_pixels(
    frame: &CapturedFrame,
    crop: Option<Rectangle<i32, Buffer>>,
) -> Result<(smithay::utils::Size<i32, Buffer>, Vec<u8>), String> {
    let rect = crop.unwrap_or_else(|| Rectangle::from_size(frame.size));
    let x0 = rect.loc.x.clamp(0, frame.size.w) as usize;
    let y0 = rect.loc.y.clamp(0, frame.size.h) as usize;
    let x1 = rect
        .loc
        .x
        .saturating_add(rect.size.w)
        .clamp(0, frame.size.w) as usize;
    let y1 = rect
        .loc
        .y
        .saturating_add(rect.size.h)
        .clamp(0, frame.size.h) as usize;
    if x1 <= x0 || y1 <= y0 {
        return Err("screenshot selection is empty".into());
    }
    let width = x1 - x0;
    let height = y1 - y0;
    let source_stride = frame.size.w as usize * 4;
    if frame.pixels.len() != source_stride * frame.size.h as usize {
        return Err("captured image has an invalid pixel buffer".into());
    }
    let row_bytes = width * 4;
    let mut pixels = Vec::with_capacity(row_bytes * height);
    for y in y0..y1 {
        let offset = y * source_stride + x0 * 4;
        pixels.extend_from_slice(&frame.pixels[offset..offset + row_bytes]);
    }
    Ok(((width as i32, height as i32).into(), pixels))
}

fn encode_png(width: u32, height: u32, pixels: &[u8]) -> Result<Vec<u8>, String> {
    let mut encoded = Vec::new();
    let mut encoder = png::Encoder::new(&mut encoded, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer
        .write_image_data(pixels)
        .map_err(|error| error.to_string())?;
    drop(writer);
    Ok(encoded)
}

fn save_png(bytes: &[u8]) -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_PICTURES_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join("Pictures"))
        })
        .ok_or_else(|| "neither XDG_PICTURES_DIR nor HOME is set".to_owned())?;
    let directory = base.join("Screenshots");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    for suffix in 0..1000 {
        let name = if suffix == 0 {
            format!("Screenshot-{timestamp}.png")
        } else {
            format!("Screenshot-{timestamp}-{suffix}.png")
        };
        let path = directory.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                if let Err(error) = file.write_all(bytes) {
                    let _ = fs::remove_file(&path);
                    return Err(format!("could not write {}: {error}", path.display()));
                }
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("could not create {}: {error}", path.display())),
        }
    }
    Err("could not allocate a unique screenshot filename".into())
}

pub(crate) fn send_selection(
    state: &mut Villain,
    mime_type: &str,
    fd: OwnedFd,
    payload: &SelectionPayload,
) {
    let SelectionPayload::Png(bytes) = payload else {
        return;
    };
    if mime_type != IMAGE_MIME {
        return;
    }
    if state.screenshot.transfers.len() >= MAX_TRANSFERS {
        tracing::warn!("screenshot clipboard transfer limit reached");
        return;
    }
    let fd_number = fd.as_raw_fd();
    // Clipboard consumers may be slow; keep all pipe writes nonblocking on the compositor loop.
    let flags = unsafe { libc::fcntl(fd_number, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd_number, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        tracing::warn!(error = %io::Error::last_os_error(), "could not prepare screenshot clipboard pipe");
        return;
    }
    let file = std::fs::File::from(fd);
    state.screenshot.next_transfer_id = state.screenshot.next_transfer_id.wrapping_add(1);
    let id = state.screenshot.next_transfer_id;
    let bytes = Arc::clone(bytes);
    let handle = state.loop_handle.clone();
    let mut offset = 0usize;
    let source = Generic::new(file, Interest::WRITE, Mode::Level);
    let io_token = match handle.insert_source(source, move |_, source, state| {
        // The event-loop callback exclusively owns this registered pipe source.
        let result = unsafe { source.get_mut() }.write(&bytes[offset..]);
        match result {
            Ok(0) if offset == bytes.len() => {
                state
                    .screenshot
                    .finish_transfer(id, &state.loop_handle.clone());
                Ok(PostAction::Remove)
            }
            Ok(0) => {
                tracing::warn!("screenshot clipboard pipe closed before transfer completed");
                state
                    .screenshot
                    .finish_transfer(id, &state.loop_handle.clone());
                Ok(PostAction::Remove)
            }
            Ok(written) => {
                offset += written;
                if offset == bytes.len() {
                    state
                        .screenshot
                        .finish_transfer(id, &state.loop_handle.clone());
                    Ok(PostAction::Remove)
                } else {
                    Ok(PostAction::Continue)
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(PostAction::Continue),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => Ok(PostAction::Continue),
            Err(error) => {
                tracing::warn!(%error, "screenshot clipboard transfer failed");
                state
                    .screenshot
                    .finish_transfer(id, &state.loop_handle.clone());
                Ok(PostAction::Remove)
            }
        }
    }) {
        Ok(token) => token,
        Err(error) => {
            tracing::warn!(%error, "could not register screenshot clipboard pipe");
            return;
        }
    };
    let timeout = match handle.insert_source(
        Timer::from_duration(TRANSFER_TIMEOUT),
        move |_, _, state| {
            if let Some(index) = state
                .screenshot
                .transfers
                .iter()
                .position(|transfer| transfer.id == id)
            {
                let transfer = state.screenshot.transfers.swap_remove(index);
                state.loop_handle.remove(transfer.io);
                tracing::warn!("screenshot clipboard transfer timed out");
            }
            TimeoutAction::Drop
        },
    ) {
        Ok(token) => token,
        Err(error) => {
            handle.remove(io_token);
            tracing::warn!(%error, "could not register screenshot clipboard timeout");
            return;
        }
    };
    state.screenshot.transfers.push(Transfer {
        id,
        io: io_token,
        timeout,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gestures_suppressed_by_screenshot_selection_stay_suppressed_until_end() {
        let mut screenshot = ScreenshotState::default();
        screenshot.intercepted_buttons.insert(BTN_LEFT);

        assert!(!screenshot.begin_pinch_gesture());
        assert!(!screenshot.update_pinch_gesture());
        screenshot.intercepted_buttons.clear();
        assert!(!screenshot.update_pinch_gesture());
        assert!(!screenshot.end_pinch_gesture());

        screenshot.intercepted_buttons.insert(BTN_LEFT);
        assert!(!screenshot.begin_hold_gesture());
        assert!(!screenshot.update_hold_gesture());
        screenshot.intercepted_buttons.clear();
        assert!(!screenshot.update_hold_gesture());
        assert!(!screenshot.end_hold_gesture());
        screenshot.clear_gesture_streams();
        assert!(screenshot.begin_hold_gesture());
        assert!(screenshot.end_hold_gesture());
    }

    #[test]
    fn forwarded_gesture_is_detected_while_active() {
        let mut screenshot = ScreenshotState::default();
        assert!(screenshot.begin_pinch_gesture());
        assert!(screenshot.has_forwarded_gesture());
        assert!(screenshot.end_pinch_gesture());
        assert!(!screenshot.has_forwarded_gesture());

        assert!(screenshot.begin_hold_gesture());
        assert!(screenshot.has_forwarded_gesture());
        assert!(screenshot.end_hold_gesture());
        assert!(!screenshot.has_forwarded_gesture());
    }
}
