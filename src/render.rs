//! The nested Winit backend and the one-frame render path.

use smithay::{
    backend::{
        input::{
            AbsolutePositionEvent, Axis, AxisSource, Event, InputEvent, KeyState, PointerAxisEvent,
            PointerButtonEvent,
        },
        renderer::{damage::OutputDamageTracker, gles::GlesRenderer},
        winit::{self, WinitEvent},
    },
    input::pointer::{AxisFrame, ButtonEvent},
    output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::calloop::EventLoop,
    utils::{Logical, Physical, Size, Transform},
};

use crate::state::Villain;
use std::time::Instant;

const SLOW_FRAME_US: u128 = 16_000;
const SLOW_FRAME_STAGE_US: u128 = 8_000;

pub struct Winit {
    backend: winit::WinitGraphicsBackend<GlesRenderer>,
    output: Output,
    damage: OutputDamageTracker,
    redraw_requested: bool,
}

impl Winit {
    pub fn request_redraw(&mut self) {
        if !self.redraw_requested {
            self.backend.window().request_redraw();
            self.redraw_requested = true;
        }
    }

    pub(crate) fn renderer(&mut self) -> &mut GlesRenderer {
        self.backend.renderer()
    }

    pub(crate) fn set_scale(
        &mut self,
        scale: f64,
    ) -> smithay::utils::Size<i32, smithay::utils::Logical> {
        self.output
            .change_current_state(None, None, Some(Scale::Fractional(scale)), None);
        output_logical_size(self.backend.window_size(), scale)
    }
}

/// Convert a physical output extent to the compositor's integer logical grid.
/// Scale may be fractional even though Smithay's typed geometry Scale is integer.
pub(crate) fn output_logical_size(size: Size<i32, Physical>, scale: f64) -> Size<i32, Logical> {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    (
        (f64::from(size.w) / scale).round() as i32,
        (f64::from(size.h) / scale).round() as i32,
    )
        .into()
}

/// Start a nested output and connect its events to the compositor loop.
pub fn init_winit(
    event_loop: &mut EventLoop<Villain>,
    state: &mut Villain,
) -> Result<(), Box<dyn std::error::Error>> {
    let (backend, winit_source) = winit::init()?;
    let output_scale = state.config.output_scale;
    // Villain renders the cursor into its nested output. Winit only supplies
    // host pointer events and hides the host cursor while it is over this window.
    backend.window().set_cursor_visible(false);
    state.output_size = output_logical_size(backend.window_size(), output_scale);
    let mode = Mode {
        size: backend.window_size(),
        refresh: 60_000,
    };

    let output = Output::new(
        "villain-output".into(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Villain".into(),
            model: "Nested Winit output".into(),
        },
    );
    let _global = output.create_global::<Villain>(&state.display_handle);
    output.change_current_state(
        Some(mode),
        Some(Transform::Flipped180),
        Some(Scale::Fractional(output_scale)),
        Some((0, 0).into()),
    );
    output.set_preferred(mode);
    state.space.map_output(&output, (0, 0));

    let damage = OutputDamageTracker::from_output(&output);
    state.winit = Some(Winit {
        backend,
        output,
        damage,
        redraw_requested: false,
    });
    event_loop
        .handle()
        .insert_source(winit_source, move |event, _, state| match event {
            WinitEvent::Resized { size, .. } => {
                state.cancel_swipe(0);
                state.cancel_workspace_motion();
                state.output_size = output_logical_size(size, state.config.output_scale);
                state.overview_panes.clear();
                state.winit.as_ref().unwrap().output.change_current_state(
                    Some(Mode {
                        size,
                        refresh: 60_000,
                    }),
                    None,
                    None,
                    None,
                );
                state.relayout_active_workspace();
                state.request_repaint();
            }
            WinitEvent::Input(InputEvent::Keyboard { event }) => {
                crate::keybinds::handle_keyboard_event(state, event);
            }
            WinitEvent::Input(InputEvent::PointerMotionAbsolute { event }) => {
                state.pointer_location = (
                    event.x() / state.config.output_scale,
                    event.y() / state.config.output_scale,
                )
                    .into();
                if crate::screenshot::pointer_motion(state) {
                    return;
                }
                state.refresh_pointer_motion(event.time_msec());
            }
            WinitEvent::Input(InputEvent::PointerButton { event }) => {
                if crate::screenshot::pointer_button(
                    state,
                    event.button_code(),
                    event.state() == smithay::backend::input::ButtonState::Pressed,
                    event.time_msec(),
                ) {
                    return;
                }
                if state.handle_compositor_button(
                    event.button_code(),
                    event.state(),
                    event.time_msec(),
                ) {
                    return;
                }
                state.refresh_pointer_and_focus(event.time_msec());
                let pointer = state.pointer.clone();
                pointer.button(
                    state,
                    &ButtonEvent {
                        serial: smithay::utils::SERIAL_COUNTER.next_serial(),
                        time: event.time_msec(),
                        button: event.button_code(),
                        state: event.state(),
                    },
                );
                pointer.frame(state);
                state.update_interaction_cursor();
            }
            WinitEvent::Input(InputEvent::PointerAxis { event }) => {
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
            WinitEvent::Focus(focused) => {
                state.host_focused = focused;
                if !focused {
                    state.cancel_swipe(0);
                    state.cancel_workspace_motion();
                    crate::screenshot::cancel(state, false);
                    state.screenshot.clear_intercepted_buttons();
                    state.release_pointer_buttons();
                    let keyboard = state.keyboard.clone();
                    for code in keyboard.pressed_keys() {
                        keyboard.input::<(), _>(
                            state,
                            code,
                            KeyState::Released,
                            smithay::utils::SERIAL_COUNTER.next_serial(),
                            0,
                            |_, _, _| smithay::input::keyboard::FilterResult::Forward,
                        );
                    }
                    state.suppressed_keys.clear();
                    state.pending_modifier = None;
                }
                if focused {
                    state.refresh_pointer_surface(0);
                    state.restore_active_workspace_focus();
                } else {
                    state.refresh_pointer(0);
                }
            }
            WinitEvent::Redraw => {
                state.request_repaint();
                render_frame(state);
            }
            WinitEvent::CloseRequested => state.loop_signal.stop(),
            _ => {}
        })?;

    Ok(())
}

fn render_frame(state: &mut Villain) {
    let Some(mut winit) = state.winit.take() else {
        return;
    };
    winit.redraw_requested = false;
    if !state.repaint_needed {
        state.winit = Some(winit);
        return;
    }
    state.repaint_needed = false;

    let profile = tracing::enabled!(target: "villain::perf", tracing::Level::DEBUG);
    let frame_started = profile.then(Instant::now);
    let mut bind_us = 0;
    let mut elements_us = 0;
    let mut cursor_us = 0;
    let mut overview_us = 0;
    let mut night_light_us = 0;
    let mut output_us = 0;
    let mut submit_us = 0;

    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let stage_started = profile.then(Instant::now);
        let age = winit.backend.buffer_age().unwrap_or(0);
        let (renderer, mut framebuffer) = winit.backend.bind()?;
        if let Some(started) = stage_started {
            bind_us = started.elapsed().as_micros();
        }
        let now = state.start_time.elapsed();
        let stage_started = profile.then(Instant::now);
        let cursor_elements = state.cursor.render_elements(
            renderer,
            state.pointer_location,
            now,
            state.config.output_scale,
        );
        if let Some(started) = stage_started {
            cursor_us = started.elapsed().as_micros();
        }
        let stage_started = profile.then(Instant::now);
        let elements = crate::overview_render::elements(state, renderer, cursor_elements)?;
        if let Some(started) = stage_started {
            overview_us = started.elapsed().as_micros();
        }
        elements_us = cursor_us + overview_us;
        let stage_started = profile.then(Instant::now);
        let elements = state
            .night_light
            .apply(renderer, state.output_size, elements)?;
        if let Some(started) = stage_started {
            night_light_us = started.elapsed().as_micros();
        }
        let stage_started = profile.then(Instant::now);
        let result = winit.damage.render_output(
            renderer,
            &mut framebuffer,
            age,
            &elements,
            [0.0, 0.0, 0.0, 1.0],
        )?;
        if let Some(started) = stage_started {
            output_us = started.elapsed().as_micros();
        }
        let damage = result.damage.cloned();
        drop(framebuffer);
        let stage_started = profile.then(Instant::now);
        if let Some(damage) = damage {
            winit.backend.submit(Some(&damage))?;
        }
        if let Some(started) = stage_started {
            submit_us = started.elapsed().as_micros();
        }
        state.schedule_frame_callbacks(&winit.output);
        let delay = state.cursor.next_animation_delay(now);
        state.schedule_cursor_frame(delay);
        Ok(())
    })();
    if let Some(started) = frame_started {
        let frame_us = started.elapsed().as_micros();
        let slow_frame = frame_us >= SLOW_FRAME_US;
        let slow_stage = [
            bind_us,
            elements_us,
            cursor_us,
            overview_us,
            night_light_us,
            output_us,
            submit_us,
        ]
        .into_iter()
        .any(|duration| duration >= SLOW_FRAME_STAGE_US);
        if slow_frame || slow_stage {
            tracing::debug!(
                target: "villain::perf",
                event = "slow_render_frame",
                frame_us,
                bind_us,
                elements_us,
                cursor_us,
                overview_us,
                night_light_us,
                output_us,
                submit_us,
                success = result.is_ok(),
                "render frame exceeded timing threshold"
            );
        }
    }
    if let Err(error) = result {
        state.cancel_swipe(0);
        state.cancel_workspace_motion();
        tracing::error!(%error, "Winit render failed");
        state.repaint_needed = true;
    }
    state.winit = Some(winit);
}
