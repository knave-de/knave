//! One owner per swipe stream; logical activation uses the shared workspace switch path.
use crate::{motion::Tween, workspace_motion::SETTLE};
use crate::{state::Villain, workspace_navigation::Direction};
use knave_config::gesture::{GestureAction, GestureBinding, GestureConfig, GestureDirection};
use smithay::{
    input::pointer::{GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent},
    utils::SERIAL_COUNTER,
};
use std::time::Instant;

pub(crate) fn bindings(config: &GestureConfig) -> Vec<GestureBinding> {
    let mut result = vec![GestureBinding {
        fingers: 3,
        direction: GestureDirection::Horizontal,
        action: GestureAction::Workspace,
    }];
    for binding in &config.bind {
        if let Some(current) = result.iter_mut().find(|current| {
            current.fingers == binding.fingers && current.direction == binding.direction
        }) {
            *current = binding.clone();
        } else {
            result.push(binding.clone());
        }
    }
    result
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Axis {
    #[default]
    Pending,
    Horizontal,
    Rejected,
}
#[derive(Clone, Copy, Debug, Default)]
struct Recognizer {
    x: f64,
    y: f64,
    axis: Axis,
}
impl Recognizer {
    fn update(&mut self, x: f64, y: f64, width: f64) -> f64 {
        if !x.is_finite() || !y.is_finite() || width <= 0.0 {
            return self.offset(width);
        }
        self.x = (self.x + x).clamp(-width / 4.0, width / 4.0);
        self.y = (self.y + y).clamp(-1e6, 1e6);
        if self.axis == Axis::Pending && self.x.abs().max(self.y.abs()) >= 8.0 {
            if self.x.abs() > self.y.abs() * 1.25 {
                self.axis = Axis::Horizontal;
            } else if self.y.abs() > self.x.abs() * 1.25 {
                self.axis = Axis::Rejected;
            }
        }
        self.offset(width)
    }
    fn offset(&self, width: f64) -> f64 {
        if self.axis == Axis::Horizontal {
            (self.x * 4.0).clamp(-width.max(0.0), width.max(0.0))
        } else {
            0.0
        }
    }
    fn commits(&self, width: f64, cancelled: bool) -> bool {
        !cancelled && width > 0.0 && self.offset(width).abs() >= width * 0.25
    }
}

enum Owner {
    Client,
    Reserved(Recognizer),
    Discard,
}
struct Stream {
    device: String,
    source: usize,
    owner: Owner,
}
#[derive(Clone, Copy)]
pub(crate) struct Transition {
    pub source: usize,
    pub target: usize,
    pub direction: Direction,
    pub offset: f64,
    target_was_occupied: bool,
    settle: Option<Tween>,
}
#[derive(Default)]
pub(crate) struct Gestures {
    stream: Option<Stream>,
    pub transition: Option<Transition>,
    transition_device: Option<String>,
}
impl Gestures {
    pub fn visible(&self, index: usize) -> bool {
        self.transition
            .is_some_and(|transition| transition.source == index || transition.target == index)
    }
    pub(crate) fn settling(&self) -> bool {
        self.transition
            .is_some_and(|transition| transition.settle.is_some())
    }
    fn reserved(&self) -> bool {
        self.stream
            .as_ref()
            .is_some_and(|stream| matches!(stream.owner, Owner::Reserved(_)))
    }
}

impl Villain {
    fn swipe_blocked(&self) -> bool {
        self.screenshot.intercepting_pointer()
            || self.pointer.is_grabbed()
            || self.overview_visible
            || !self.host_focused
            || self.tty.as_ref().is_some_and(|tty| !tty.is_active())
    }
    pub(crate) fn stop_swipe_visual(&mut self) {
        self.gestures.transition_device = None;
        if self.gestures.transition.take().is_some() {
            self.request_repaint();
            self.trim_dynamic_workspaces();
        }
        self.retire_motion_timer();
    }
    /// Keep a tombstone until End so an interrupted reserved stream cannot leak to clients.
    pub(crate) fn cancel_swipe(&mut self, time: u32) {
        let forward = self
            .gestures
            .stream
            .as_ref()
            .is_some_and(|stream| matches!(stream.owner, Owner::Client));
        if forward {
            self.pointer.clone().gesture_swipe_end(
                self,
                &GestureSwipeEndEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time,
                    cancelled: true,
                },
            );
        }
        if let Some(stream) = &mut self.gestures.stream {
            stream.owner = Owner::Discard;
        }
        self.stop_swipe_visual();
    }
    pub(crate) fn maintain_swipe(&mut self) {
        if self.workspace_motion.active() && self.workspace_motion_blocked() {
            self.cancel_workspace_motion();
        }
        let target_disappeared = self.config.workspace.skip_empty
            && self.gestures.reserved()
            && self.gestures.transition.is_some_and(|transition| {
                transition.target_was_occupied && !self.workspace_occupied(transition.target)
            });
        if target_disappeared
            || ((self.gestures.reserved()
                || self.gestures.transition.is_some()
                || self.screenshot.intercepting_pointer())
                && self.swipe_blocked())
        {
            self.cancel_swipe(0);
        }
    }
    pub(crate) fn swipe_device_removed(&mut self, device: &str) {
        if self
            .gestures
            .stream
            .as_ref()
            .is_some_and(|stream| stream.device == device)
            || self.gestures.transition_device.as_deref() == Some(device)
        {
            self.cancel_swipe(0);
            self.gestures.stream = None;
        }
    }
    pub(crate) fn swipe_begin(&mut self, device: String, fingers: u32, time: u32) {
        if self
            .gestures
            .stream
            .as_ref()
            .is_some_and(|stream| stream.device != device)
        {
            return;
        }
        self.cancel_swipe(time);
        let configured = self.config.gestures.iter().any(|binding| {
            binding.fingers == fingers && binding.action == GestureAction::Workspace
        });
        let owner = if self.screenshot.intercepting_pointer() {
            Owner::Discard
        } else if configured && !self.swipe_blocked() {
            Owner::Reserved(Recognizer::default())
        } else {
            self.pointer.clone().gesture_swipe_begin(
                self,
                &GestureSwipeBeginEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time,
                    fingers,
                },
            );
            Owner::Client
        };
        if matches!(owner, Owner::Reserved(_)) {
            self.cancel_workspace_motion();
        }
        self.gestures.stream = Some(Stream {
            device,
            source: self.active_workspace,
            owner,
        });
    }
    pub(crate) fn swipe_update(&mut self, device: &str, x: f64, y: f64, time: u32) {
        self.maintain_swipe();
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        let Some(stream) = self
            .gestures
            .stream
            .as_mut()
            .filter(|stream| stream.device == device)
        else {
            return;
        };
        match &mut stream.owner {
            Owner::Client => self.pointer.clone().gesture_swipe_update(
                self,
                &GestureSwipeUpdateEvent {
                    time,
                    delta: (x, y).into(),
                },
            ),
            Owner::Reserved(recognizer) => {
                let offset = recognizer.update(x, y, f64::from(self.output_size.w));
                if offset != 0.0 || self.gestures.transition.is_some() {
                    let source = stream.source;
                    let previous = self.gestures.transition;
                    let direction = if offset < 0.0 {
                        Direction::Next
                    } else if offset > 0.0 {
                        Direction::Previous
                    } else {
                        previous.expect("existing preview at zero offset").direction
                    };
                    // Keep the preview stable while windows appear in intervening slots.
                    let target = previous
                        .filter(|preview| {
                            preview.source == source && preview.direction == direction
                        })
                        .map(|preview| preview.target)
                        .or_else(|| self.workspace_target(source, direction));
                    let Some(target) = target else {
                        self.stop_swipe_visual();
                        return;
                    };
                    if !self.ensure_workspace_exists(target) {
                        self.stop_swipe_visual();
                        return;
                    }
                    self.gestures.transition_device = Some(device.into());
                    self.gestures.transition = Some(Transition {
                        source,
                        target,
                        direction,
                        offset,
                        target_was_occupied: previous
                            .filter(|preview| {
                                preview.source == source && preview.direction == direction
                            })
                            .map_or_else(
                                || self.workspace_occupied(target),
                                |preview| preview.target_was_occupied,
                            ),
                        settle: None,
                    });
                    self.request_repaint();
                }
            }
            Owner::Discard => {}
        }
    }
    pub(crate) fn swipe_end(&mut self, device: &str, cancelled: bool, time: u32) {
        self.maintain_swipe();
        if !self
            .gestures
            .stream
            .as_ref()
            .is_some_and(|stream| stream.device == device)
        {
            return;
        }
        let stream = self.gestures.stream.take().expect("matching stream");
        match stream.owner {
            Owner::Client => self.pointer.clone().gesture_swipe_end(
                self,
                &GestureSwipeEndEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time,
                    cancelled,
                },
            ),
            Owner::Reserved(recognizer) => {
                let Some(mut transition) = self.gestures.transition.take() else {
                    return;
                };
                let width = f64::from(self.output_size.w);
                let commit = recognizer.commits(width, cancelled);
                let end = if commit {
                    width.copysign(transition.offset)
                } else {
                    0.0
                };
                if commit {
                    // The same activation/focus/history path, without starting a second animation.
                    self.switch_workspace_immediate(transition.target);
                }
                transition.settle =
                    Some(Tween::new(transition.offset, end, Instant::now(), SETTLE));
                self.gestures.transition_device = Some(stream.device);
                self.gestures.transition = Some(transition);
                self.trim_dynamic_workspaces();
                self.request_repaint();
                self.schedule_motion_tick();
            }
            Owner::Discard => {}
        }
    }
    pub(crate) fn advance_swipe_settle(&mut self, now: Instant) -> bool {
        let Some(transition) = &mut self.gestures.transition else {
            return false;
        };
        let Some(tween) = transition.settle else {
            return false;
        };
        let sample = tween.sample(now);
        transition.offset = sample.value;
        if sample.finished {
            self.gestures.transition = None;
            self.gestures.transition_device = None;
            self.trim_dynamic_workspaces();
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    #[ignore = "requires private Unix sockets and an isolated XDG_RUNTIME_DIR"]
    fn gesture_workspace_lifecycle_and_timer_cleanup() {
        use smithay::reexports::{calloop::EventLoop, wayland_server::Display};
        let mut event_loop = EventLoop::try_new().unwrap();
        let mut state = Villain::new(
            &mut event_loop,
            Display::new().unwrap(),
            crate::config::RuntimeConfig::load().unwrap(),
        );
        state.output_size = (1280, 800).into();
        state.swipe_begin("pad".into(), 3, 1);
        state.swipe_update("other", -200.0, 0.0, 2);
        state.swipe_end("other", false, 3);
        assert!(state.gestures.transition.is_none());
        state.swipe_update("pad", -100.0, 0.0, 4);
        assert_eq!(state.active_workspace, 0);
        assert_eq!(state.gestures.transition.unwrap().offset, -400.0);
        state.swipe_end("pad", false, 5);
        assert_eq!(state.active_workspace, 1);
        assert!(state.workspace_motion.timer.is_some());
        let deadline = Instant::now() + Duration::from_secs(1);
        while state.gestures.transition.is_some() {
            assert!(Instant::now() < deadline);
            event_loop
                .dispatch(Duration::from_millis(20), &mut state)
                .unwrap();
        }
        assert!(state.workspace_motion.timer.is_none());
        state.swipe_begin("pad".into(), 3, 6);
        state.swipe_update("pad", -100.0, 0.0, 7);
        state.swipe_end("pad", false, 8);
        assert_eq!(state.active_workspace, 2);
        state.swipe_device_removed("pad");
        assert!(state.gestures.transition.is_none());
        assert!(state.workspace_motion.timer.is_none());
        state.switch_workspace(1);
        state.swipe_begin("pad".into(), 3, 6);
        state.swipe_update("pad", -100.0, 0.0, 7);
        state.swipe_end("pad", true, 8);
        assert_eq!(state.active_workspace, 1);
        state.swipe_device_removed("pad");
        assert!(state.gestures.transition.is_none());
        assert!(state.workspace_motion.timer.is_none());
        state.swipe_begin("pad".into(), 3, 9);
        state.swipe_update("pad", -100.0, 0.0, 10);
        state.host_focused = false;
        state.maintain_swipe();
        state.host_focused = true;
        state.swipe_update("pad", -100.0, 0.0, 11);
        state.swipe_end("pad", false, 12);
        assert_eq!(state.active_workspace, 1);
        assert!(state.gestures.transition.is_none());
    }
    #[test]
    fn default_overrides_merge_by_identity_and_later_wins() {
        let mut config = GestureConfig {
            bind: vec![GestureBinding {
                fingers: 4,
                direction: GestureDirection::Horizontal,
                action: GestureAction::Workspace,
            }],
        };
        assert_eq!(bindings(&config).len(), 2);
        config.bind.push(GestureBinding {
            fingers: 3,
            direction: GestureDirection::Horizontal,
            action: GestureAction::None,
        });
        assert_eq!(bindings(&config)[0].action, GestureAction::None);
        config.bind.push(GestureBinding {
            fingers: 3,
            direction: GestureDirection::Horizontal,
            action: GestureAction::Workspace,
        });
        assert_eq!(bindings(&config)[0].action, GestureAction::Workspace);
    }
    #[test]
    fn thresholds_reversal_clamping_and_cancellation() {
        let mut recognizer = Recognizer::default();
        assert_eq!(recognizer.update(-7.0, 0.0, 1000.0), 0.0);
        assert_eq!(recognizer.update(-1.0, 0.0, 1000.0), -32.0);
        assert!(!recognizer.commits(1000.0, false));
        recognizer.update(-54.5, 0.0, 1000.0);
        assert!(recognizer.commits(1000.0, false));
        assert!(!recognizer.commits(1000.0, true));
        assert_eq!(recognizer.update(400.0, 0.0, 1000.0), 1000.0);
        assert_eq!(recognizer.update(-20.0, 0.0, 1000.0), 920.0);
        assert_eq!(recognizer.update(f64::NAN, 0.0, 1000.0), 920.0);
    }
    #[test]
    fn vertical_dominance_rejects_until_end() {
        let mut recognizer = Recognizer::default();
        assert_eq!(recognizer.update(2.0, 10.0, 1000.0), 0.0);
        assert_eq!(recognizer.update(200.0, 0.0, 1000.0), 0.0);
        assert!(!recognizer.commits(1000.0, false));
        let mut diagonal = Recognizer::default();
        assert_eq!(diagonal.update(10.0, 10.0, 1000.0), 0.0);
        assert_eq!(diagonal.update(5.0, 0.0, 1000.0), 60.0);
    }
}
