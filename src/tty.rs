//! Direct Linux output: libseat owns access, GBM allocates buffers, DRM scans
//! them out. One connected connector/CRTC is used for this learning backend.
use crate::state::Villain;
use smithay::{
    backend::{
        allocator::{
            Fourcc,
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        },
        drm::{DrmDevice, DrmDeviceFd, DrmEvent, GbmBufferedSurface},
        egl::{EGLContext, EGLDisplay},
        input::{
            AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputEvent, KeyState,
            PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
        },
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{Bind, damage::OutputDamageTracker, gles::GlesRenderer},
        session::{Event as SessionEvent, Session, libseat::LibSeatSession},
        udev::{all_gpus, primary_gpu},
    },
    input::pointer::{AxisFrame, ButtonEvent},
    output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::{
        calloop::EventLoop,
        drm::control::{Device, ModeTypeFlags, connector},
        input::Libinput,
        rustix::fs::OFlags,
    },
    utils::{DeviceFd, SERIAL_COUNTER, Transform},
};
use std::{error::Error, path::PathBuf};

type BufferedSurface = GbmBufferedSurface<GbmAllocator<DrmDeviceFd>, ()>;

pub struct Tty {
    // Display resources drop before the event loop's libseat notifier.
    surface: BufferedSurface,
    pub renderer: GlesRenderer,
    drm: DrmDevice,
    output: Output,
    damage: OutputDamageTracker,
    active: bool,
    pending: bool,
    pub session: LibSeatSession,
    input_devices: Vec<smithay::reexports::input::Device>,
}

impl Tty {
    pub(crate) fn set_scale(
        &mut self,
        scale: f64,
    ) -> Option<smithay::utils::Size<i32, smithay::utils::Logical>> {
        let mode = self.output.current_mode()?;
        self.output
            .change_current_state(None, None, Some(Scale::Fractional(scale)), None);
        Some(crate::render::output_logical_size(mode.size, scale))
    }
}

pub fn init(
    event_loop: &mut EventLoop<Villain>,
    state: &mut Villain,
) -> Result<(), Box<dyn Error>> {
    tracing::info!("acquiring Linux seat for direct TTY backend");
    let (mut session, notifier) = LibSeatSession::new().map_err(|error| format!("Cannot acquire Linux seat: {error}. Run from a logged-in local TTY with logind or seatd available."))?;
    let seat = session.seat();
    if !session.is_active() {
        return Err("Linux seat is not active; run from the active local TTY".into());
    }
    tracing::info!("Linux seat acquired and active");
    tracing::info!("selecting DRM device for direct TTY backend");
    let path = match std::env::var_os("VILLAIN_DRM_DEVICE") {
        Some(path) => PathBuf::from(path),
        None => primary_gpu(&seat)?
            .or_else(|| all_gpus(&seat).ok()?.into_iter().next())
            .ok_or("No DRM GPU found on this seat")?,
    };
    tracing::info!(gpu = %path.display(), "opening DRM device through Linux seat");
    let fd = session.open(&path, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NONBLOCK)?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let device_id = fd.dev_id()?;
    tracing::info!(gpu = %path.display(), "initializing DRM device and querying connectors");
    let (mut drm, drm_notifier) = DrmDevice::new(fd.clone(), false)?;
    let resources = drm.resource_handles()?;
    let mut selected = None;
    'connectors: for handle in resources.connectors() {
        let info = drm.get_connector(*handle, true)?;
        if info.state() != connector::State::Connected {
            continue;
        }
        let Some(mode) = info
            .modes()
            .iter()
            .find(|mode| mode.mode_type().contains(ModeTypeFlags::PREFERRED))
            .or_else(|| info.modes().first())
            .copied()
        else {
            continue;
        };
        for encoder in info.encoders() {
            let encoder = drm.get_encoder(*encoder)?;
            for crtc in resources.filter_crtcs(encoder.possible_crtcs()) {
                if let Ok(surface) = drm.create_surface(crtc, mode, &[*handle]) {
                    selected = Some((surface, mode, info));
                    break 'connectors;
                }
            }
        }
    }
    let (surface, mode, connector) = selected
        .ok_or("No connected display with a usable CRTC; connect a monitor before starting")?;
    tracing::info!(
        gpu = %path.display(),
        connector = ?connector.interface(),
        mode = ?mode.size(),
        "selected direct display mode"
    );
    tracing::info!(gpu = %path.display(), "initializing GBM device");
    let gbm = GbmDevice::new(fd)?;
    // EGL retains the cloned GBM native display. All rendering stays on the
    // event-loop thread, which owns the context for its complete lifetime.
    tracing::info!(gpu = %path.display(), "initializing EGL display and context");
    let egl = unsafe { EGLDisplay::new(gbm.clone())? };
    let context = EGLContext::new(&egl)?;
    tracing::info!(gpu = %path.display(), "initializing GLES renderer");
    let renderer = unsafe { GlesRenderer::new(context)? };
    // Advertise GPU-buffer import to EGL clients such as Kitty.
    use smithay::{backend::renderer::ImportDma, wayland::dmabuf::DmabufFeedbackBuilder};
    let feedback = DmabufFeedbackBuilder::new(device_id, renderer.dmabuf_formats()).build()?;
    state
        .dmabuf_state
        .create_global_with_default_feedback::<Villain>(&state.display_handle, &feedback);
    let formats = renderer
        .egl_context()
        .dmabuf_render_formats()
        .iter()
        .copied()
        .collect::<Vec<_>>();
    let allocator = GbmAllocator::new(gbm, GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT);
    tracing::info!(gpu = %path.display(), "allocating direct display buffers");
    let surface = GbmBufferedSurface::new(
        surface,
        allocator,
        &[Fourcc::Argb8888, Fourcc::Abgr8888],
        formats,
    )?;
    let size = (i32::from(mode.size().0), i32::from(mode.size().1));
    let output_scale = state.config.output_scale;
    let output = Output::new(
        format!("{:?}-{}", connector.interface(), connector.interface_id()),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Villain".into(),
            model: "DRM/KMS".into(),
        },
    );
    output.create_global::<Villain>(&state.display_handle);
    let wl_mode = Mode {
        size: size.into(),
        refresh: mode.vrefresh() as i32 * 1000,
    };
    output.change_current_state(
        Some(wl_mode),
        Some(Transform::Normal),
        Some(Scale::Fractional(output_scale)),
        Some((0, 0).into()),
    );
    output.set_preferred(wl_mode);
    state.output_size = crate::render::output_logical_size(size.into(), output_scale);
    state.pointer_location = (
        f64::from(state.output_size.w) / 2.0,
        f64::from(state.output_size.h) / 2.0,
    )
        .into();
    state.space.map_output(&output, (0, 0));
    let damage = OutputDamageTracker::from_output(&output);
    state.tty = Some(Tty {
        surface,
        renderer,
        drm,
        output,
        damage,
        active: true,
        pending: false,
        session: session.clone(),
        input_devices: Vec::new(),
    });

    let mut input =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.into());
    tracing::info!("assigning libinput to the active Linux seat");
    input
        .udev_assign_seat(&seat)
        .map_err(|_| "libinput could not assign the seat")?;
    event_loop.handle().insert_source(
        LibinputInputBackend::new(input.clone()),
        |event, _, state| process_input(event, state),
    )?;
    event_loop
        .handle()
        .insert_source(drm_notifier, |event, _, state| {
            if let Some(tty) = state.tty.as_mut() {
                match event {
                    DrmEvent::VBlank(_) => {
                        if let Err(error) = tty.surface.frame_submitted() {
                            tracing::error!(%error, "page flip completion failed");
                        }
                        tty.pending = false;
                    }
                    DrmEvent::Error(error) => {
                        // Revoking DRM master during a VT switch can invalidate
                        // an outstanding page flip. This is recoverable.
                        tracing::warn!(%error, "DRM event failed; scheduling a fresh frame");
                        tty.pending = false;
                        tty.surface.reset_buffers();
                        state.request_repaint();
                    }
                }
            }
        })?;
    event_loop
        .handle()
        .insert_source(notifier, move |event, _, state| match event {
            SessionEvent::PauseSession => {
                state.cancel_swipe(0);
                state.cancel_workspace_motion();
                input.suspend();
                state.release_pointer_buttons();
                let keyboard = state.keyboard.clone();
                for code in keyboard.pressed_keys() {
                    keyboard.input::<(), _>(
                        state,
                        code,
                        KeyState::Released,
                        SERIAL_COUNTER.next_serial(),
                        0,
                        |_, _, _| smithay::input::keyboard::FilterResult::Forward,
                    );
                }
                state.suppressed_keys.clear();
                state.pending_modifier = None;
                state.host_focused = false;
                state.refresh_pointer(0);
                if let Some(tty) = state.tty.as_mut() {
                    tty.active = false;
                    tty.drm.pause();
                    // A VT switch can prevent delivery of the last flip event.
                    // Retire only a frame Villain actually submitted.
                    if tty.pending
                        && let Err(error) = tty.surface.frame_submitted()
                    {
                        tracing::debug!(%error, "could not retire paused DRM frame");
                    }
                    tty.pending = false;
                    tty.surface.reset_buffers();
                }
                tracing::info!("TTY session paused");
            }
            SessionEvent::ActivateSession => {
                if let Err(error) = input.resume() {
                    // Smithay's reference backend keeps the recovered display
                    // alive even if an input device fails to resume.
                    tracing::warn!(?error, "input resume failed");
                }
                let tty = state.tty.as_mut().unwrap();
                // Preserve connector state. Resetting every connector here
                // invalidates the existing DRM surface during VT handoff.
                if let Err(error) = tty.drm.activate(false) {
                    tracing::error!(%error, "DRM resume failed; waiting for another activation");
                    return;
                }
                tty.surface.reset_buffers();
                tty.pending = false;
                tty.active = true;
                tty.damage = OutputDamageTracker::from_output(&tty.output);
                state.host_focused = true;
                state.refresh_pointer_surface(0);
                state.restore_active_workspace_focus();
                state.request_repaint();
                tracing::info!("TTY session resumed");
            }
        })?;
    tracing::info!("DRM, renderer, and input sources registered");
    tracing::info!(gpu = %path.display(), ?size, "TTY backend ready; Ctrl+Alt+Backspace exits, Ctrl+Alt+F1–F12 switches VT");
    Ok(())
}

impl Tty {
    pub fn is_active(&self) -> bool {
        self.active
    }

    fn render(&mut self, state: &mut Villain) -> Result<(), Box<dyn Error>> {
        let (mut buffer, age) = self.surface.next_buffer()?;
        let mut framebuffer = self.renderer.bind(&mut buffer)?;
        let now = state.start_time.elapsed();
        let cursor_elements = state.cursor.render_elements(
            &mut self.renderer,
            state.pointer_location,
            now,
            state.config.output_scale,
        );
        let elements =
            crate::overview_render::elements(state, &mut self.renderer, cursor_elements)?;
        let elements = state
            .night_light
            .apply(&mut self.renderer, state.output_size, elements)?;
        let result = self.damage.render_output(
            &mut self.renderer,
            &mut framebuffer,
            usize::from(age),
            &elements,
            [0.0, 0.0, 0.0, 1.0],
        )?;
        let sync = result.sync.clone();
        let damage = result.damage.cloned();
        drop(framebuffer);
        if let Some(damage) = damage {
            self.surface.queue_buffer(Some(sync), Some(damage), ())?;
            self.pending = true;
        }
        state.schedule_frame_callbacks(&self.output);
        let delay = state.cursor.next_animation_delay(now);
        state.schedule_cursor_frame(delay);
        Ok(())
    }
}

pub fn render_if_needed(state: &mut Villain) {
    let Some(mut tty) = state.tty.take() else {
        return;
    };
    if !tty.active || tty.pending {
        state.tty = Some(tty);
        return;
    }
    state.repaint_needed = false;
    if let Err(error) = tty.render(state) {
        // The first frame after regaining DRM master can race the kernel
        // handoff. Keep the damage pending for the next real event.
        state.cancel_swipe(0);
        state.cancel_workspace_motion();
        tracing::warn!(%error, "DRM render failed; resetting buffers");
        tty.pending = false;
        tty.surface.reset_buffers();
        state.repaint_needed = true;
    }
    state.tty = Some(tty);
}

fn process_input(event: InputEvent<LibinputInputBackend>, state: &mut Villain) {
    match event {
        InputEvent::DeviceAdded { mut device } => {
            configure_input_device(&mut device, state.config.input, &state.config.devices);
            if let Some(tty) = state.tty.as_mut() {
                tty.input_devices.push(device);
            }
        }
        InputEvent::DeviceRemoved { device } => {
            state.release_pointer_buttons();
            state.screenshot.clear_gesture_streams();
            state.swipe_device_removed(&smithay::backend::input::Device::id(&device));
            if let Some(tty) = state.tty.as_mut() {
                tty.input_devices.retain(|candidate| candidate != &device);
            }
        }
        InputEvent::Keyboard { event } => crate::keybinds::handle_keyboard_event(state, event),
        InputEvent::PointerMotion { event } => {
            let delta = event.delta();
            state.pointer_location.x += delta.x / state.config.output_scale;
            state.pointer_location.y += delta.y / state.config.output_scale;
            state.pointer_location.x = state
                .pointer_location
                .x
                .clamp(0.0, f64::from(state.output_size.w - 1));
            state.pointer_location.y = state
                .pointer_location
                .y
                .clamp(0.0, f64::from(state.output_size.h - 1));
            if crate::screenshot::pointer_motion(state) {
                return;
            }
            state.refresh_pointer_motion(event.time_msec());
        }
        InputEvent::PointerMotionAbsolute { event } => {
            state.pointer_location = (
                event.x_transformed(state.output_size.w),
                event.y_transformed(state.output_size.h),
            )
                .into();
            if crate::screenshot::pointer_motion(state) {
                return;
            }
            state.refresh_pointer_motion(event.time_msec());
        }
        InputEvent::PointerButton { event } => {
            if crate::screenshot::pointer_button(
                state,
                event.button_code(),
                event.state() == ButtonState::Pressed,
                event.time_msec(),
            ) {
                return;
            }
            if state.handle_compositor_button(event.button_code(), event.state(), event.time_msec())
            {
                return;
            }
            state.refresh_pointer_and_focus(event.time_msec());
            let pointer = state.pointer.clone();
            pointer.button(
                state,
                &ButtonEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time: event.time_msec(),
                    button: event.button_code(),
                    state: event.state(),
                },
            );
            pointer.frame(state);
            state.update_interaction_cursor();
        }
        InputEvent::PointerAxis { event } => {
            state.finish_workspace_motion();
            if state.screenshot.intercepting_pointer() {
                return;
            }
            let mut frame = AxisFrame::new(event.time_msec()).source(event.source());
            for axis in [Axis::Horizontal, Axis::Vertical] {
                let amount = event
                    .amount(axis)
                    .unwrap_or_else(|| event.amount_v120(axis).unwrap_or(0.0) * 15.0 / 120.0);
                frame = frame
                    .value(axis, amount)
                    .relative_direction(axis, event.relative_direction(axis));
                if let Some(steps) = event.amount_v120(axis) {
                    frame = frame.v120(axis, steps as i32);
                }
                if event.source() == AxisSource::Finger && event.amount(axis) == Some(0.0) {
                    frame = frame.stop(axis);
                }
            }
            let pointer = state.pointer.clone();
            pointer.axis(state, frame);
            pointer.frame(state);
        }
        InputEvent::GestureSwipeBegin { event } => {
            use smithay::backend::input::{Device, GestureBeginEvent};
            state.swipe_begin(event.device().id(), event.fingers(), event.time_msec());
        }
        InputEvent::GestureSwipeUpdate { event } => {
            use smithay::backend::input::{Device, GestureSwipeUpdateEvent};
            state.swipe_update(
                &event.device().id(),
                event.delta_x() / state.config.output_scale,
                event.delta_y() / state.config.output_scale,
                event.time_msec(),
            );
        }
        InputEvent::GestureSwipeEnd { event } => {
            use smithay::backend::input::{Device, GestureEndEvent};
            state.swipe_end(&event.device().id(), event.cancelled(), event.time_msec());
        }
        InputEvent::GesturePinchBegin { event } => {
            if !state.screenshot.begin_pinch_gesture() {
                return;
            }
            use smithay::backend::input::GestureBeginEvent;
            state.pointer.clone().gesture_pinch_begin(
                state,
                &smithay::input::pointer::GesturePinchBeginEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time: event.time_msec(),
                    fingers: event.fingers(),
                },
            );
        }
        InputEvent::GesturePinchUpdate { event } => {
            if !state.screenshot.update_pinch_gesture() {
                return;
            }
            use smithay::backend::input::GesturePinchUpdateEvent;
            state.pointer.clone().gesture_pinch_update(
                state,
                &smithay::input::pointer::GesturePinchUpdateEvent {
                    time: event.time_msec(),
                    delta: (
                        event.delta_x() / state.config.output_scale,
                        event.delta_y() / state.config.output_scale,
                    )
                        .into(),
                    scale: event.scale(),
                    rotation: event.rotation(),
                },
            );
        }
        InputEvent::GesturePinchEnd { event } => {
            if !state.screenshot.end_pinch_gesture() {
                return;
            }
            use smithay::backend::input::GestureEndEvent;
            state.pointer.clone().gesture_pinch_end(
                state,
                &smithay::input::pointer::GesturePinchEndEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time: event.time_msec(),
                    cancelled: event.cancelled(),
                },
            );
        }
        InputEvent::GestureHoldBegin { event } => {
            if !state.screenshot.begin_hold_gesture() {
                return;
            }
            use smithay::backend::input::GestureBeginEvent;
            state.pointer.clone().gesture_hold_begin(
                state,
                &smithay::input::pointer::GestureHoldBeginEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time: event.time_msec(),
                    fingers: event.fingers(),
                },
            );
        }
        InputEvent::GestureHoldEnd { event } => {
            if !state.screenshot.end_hold_gesture() {
                return;
            }
            use smithay::backend::input::GestureEndEvent;
            state.pointer.clone().gesture_hold_end(
                state,
                &smithay::input::pointer::GestureHoldEndEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time: event.time_msec(),
                    cancelled: event.cancelled(),
                },
            );
        }
        _ => {}
    }
}

fn configure_input_device(
    device: &mut smithay::reexports::input::Device,
    global: crate::config::InputConfig,
    rules: &[knave_config::CompositorDeviceConfig],
) {
    if device.config_tap_finger_count() > 0
        && let Err(error) = device.config_tap_set_enabled(global.tap_to_click)
    {
        tracing::warn!(
            device = device.name(),
            ?error,
            "could not configure tap-to-click"
        );
    }
    if device.config_scroll_has_natural_scroll()
        && let Err(error) = device.config_scroll_set_natural_scroll_enabled(global.natural_scroll)
    {
        tracing::warn!(
            device = device.name(),
            ?error,
            "could not configure natural scrolling"
        );
    }
    // Device rules layer over the global policy in file order; later matches win.
    let name = device.name().to_owned();
    crate::input_devices::set_enabled(device, crate::input_devices::resolved_enabled(rules, &name));
    for rule in rules {
        if crate::input_devices::matches(&rule.name, &name) {
            crate::input_devices::apply(device, rule);
        }
    }
}

impl Villain {
    /// Apply configured device settings and virtual-keyboard client policy.
    pub fn apply_input_config(&mut self) {
        let global = self.config.input;
        let rules = self.config.devices.clone();
        let was_allowed = self.virtual_keyboard_allowed.swap(
            global.allow_virtual_keyboard,
            std::sync::atomic::Ordering::Relaxed,
        );
        if was_allowed != global.allow_virtual_keyboard {
            if global.allow_virtual_keyboard {
                self.display_handle
                    .remove_global::<Self>(self.virtual_keyboard_manager_state.global());
                let policy = std::sync::Arc::clone(&self.virtual_keyboard_allowed);
                self.virtual_keyboard_manager_state =
                    smithay::wayland::virtual_keyboard::VirtualKeyboardManagerState::new::<Self, _>(
                        &self.display_handle,
                        move |_| policy.load(std::sync::atomic::Ordering::Relaxed),
                    );
            } else {
                self.display_handle
                    .disable_global::<Self>(self.virtual_keyboard_manager_state.global());
            }
        }
        if let Some(tty) = self.tty.as_mut() {
            for device in &mut tty.input_devices {
                configure_input_device(device, global, &rules);
            }
        }
    }

    /// Snapshot the libinput devices for the desktop `devices` query.
    pub fn device_info(&self) -> Vec<knave_desktop_api::DeviceSummary> {
        use smithay::reexports::input::{DeviceCapability, SendEventsMode};

        const CAPABILITIES: [(DeviceCapability, &str); 7] = [
            (DeviceCapability::Keyboard, "keyboard"),
            (DeviceCapability::Pointer, "pointer"),
            (DeviceCapability::Touch, "touch"),
            (DeviceCapability::TabletTool, "tablet-tool"),
            (DeviceCapability::TabletPad, "tablet-pad"),
            (DeviceCapability::Gesture, "gesture"),
            (DeviceCapability::Switch, "switch"),
        ];

        let Some(tty) = self.tty.as_ref() else {
            // The nested Winit backend has no libinput seat.
            return Vec::new();
        };
        tty.input_devices
            .iter()
            .map(|device| knave_desktop_api::DeviceSummary {
                name: device.name().to_owned(),
                enabled: !device
                    .config_send_events_mode()
                    .contains(SendEventsMode::DISABLED),
                capabilities: CAPABILITIES
                    .iter()
                    .filter(|(capability, _)| device.has_capability(*capability))
                    .map(|(_, label)| (*label).to_owned())
                    .collect(),
                matched_rule: self
                    .config
                    .devices
                    .iter()
                    .rposition(|rule| crate::input_devices::matches(&rule.name, device.name())),
            })
            .collect()
    }
}
