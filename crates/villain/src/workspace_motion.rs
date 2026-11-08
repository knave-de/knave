//! Workspace presentation independent of logical activation, focus and layout.
use crate::{motion::Tween, state::Villain, workspace_navigation::Direction};
use smithay::reexports::calloop::{
    RegistrationToken,
    timer::{TimeoutAction, Timer},
};
use std::time::{Duration, Instant};

pub(crate) const SETTLE: Duration = Duration::from_millis(160);

struct Track {
    workspace: usize,
    offset: Tween,
}
#[derive(Default)]
pub(crate) struct WorkspaceMotion {
    tracks: Vec<Track>,
    pub(crate) timer: Option<RegistrationToken>,
}
impl WorkspaceMotion {
    pub fn active(&self) -> bool {
        !self.tracks.is_empty()
    }
    pub fn visible(&self, workspace: usize) -> bool {
        self.tracks.iter().any(|track| track.workspace == workspace)
    }
    pub(crate) fn highest_workspace(&self) -> Option<usize> {
        self.tracks.iter().map(|track| track.workspace).max()
    }
    pub fn scenes(&self, now: Instant) -> Vec<(usize, f64)> {
        self.tracks
            .iter()
            .map(|track| (track.workspace, track.offset.sample(now).value))
            .collect()
    }
    fn start(
        &mut self,
        source: usize,
        target: usize,
        direction: Direction,
        width: i32,
        now: Instant,
    ) {
        let distance = f64::from(direction.incoming_offset(width));
        let source_offset = self
            .tracks
            .iter()
            .find(|track| track.workspace == source)
            .map_or(0.0, |track| track.offset.sample(now).value);
        if !self.visible(source) {
            self.tracks.push(Track {
                workspace: source,
                offset: Tween::new(0.0, -distance, now, SETTLE),
            });
        }
        for track in &mut self.tracks {
            if track.workspace == source {
                track.offset.retarget(-distance, now);
            }
        }
        if let Some(index) = self
            .tracks
            .iter()
            .position(|track| track.workspace == target)
        {
            let mut track = self.tracks.remove(index);
            track.offset.retarget(0.0, now);
            self.tracks.insert(0, track);
        } else {
            // Adjacent to the departing scene even when an earlier switch is mid-flight.
            self.tracks.insert(
                0,
                Track {
                    workspace: target,
                    offset: Tween::new(source_offset + distance, 0.0, now, SETTLE),
                },
            );
        }
    }
    fn advance(&mut self, active: usize, now: Instant) -> bool {
        let changed = self.active();
        self.tracks
            .retain(|track| track.workspace == active || !track.offset.sample(now).finished);
        if self
            .tracks
            .iter()
            .all(|track| track.offset.sample(now).finished)
        {
            self.tracks.clear();
        }
        changed
    }
}

impl Villain {
    pub(crate) fn workspace_scene_visible(&self, workspace: usize) -> bool {
        self.gestures.visible(workspace) || self.workspace_motion.visible(workspace)
    }
    pub(crate) fn workspace_motion_blocked(&self) -> bool {
        self.screenshot.intercepting_pointer()
            || self.pointer.is_grabbed()
            || self.overview_visible
            || self.overview_mapped()
            || !self.host_focused
            || self.tty.as_ref().is_some_and(|tty| !tty.is_active())
    }
    pub(crate) fn timed_workspace_motion(&self) -> bool {
        self.workspace_motion.active() || self.gestures.settling()
    }
    pub(crate) fn cancel_workspace_motion(&mut self) {
        if self.workspace_motion.active() {
            self.workspace_motion.tracks.clear();
            self.request_repaint();
            self.refresh_pointer_surface(0);
        }
        self.retire_motion_timer();
        self.trim_dynamic_workspaces();
    }
    pub(crate) fn finish_workspace_motion(&mut self) {
        if self.timed_workspace_motion() {
            self.cancel_workspace_motion();
            self.stop_swipe_visual();
            self.refresh_pointer_surface(0);
        }
    }
    pub(crate) fn retire_motion_timer(&mut self) {
        if !self.timed_workspace_motion()
            && let Some(timer) = self.workspace_motion.timer.take()
        {
            self.loop_handle.remove(timer);
        }
    }
    pub(crate) fn start_workspace_motion(
        &mut self,
        source: usize,
        target: usize,
        direction: Direction,
    ) {
        if (self.winit.is_none() && self.tty.is_none())
            || self.workspace_motion_blocked()
            || self.output_size.w <= 0
        {
            return;
        }
        self.workspace_motion.start(
            source,
            target,
            direction,
            self.output_size.w,
            Instant::now(),
        );
        debug_assert!(self.workspace_motion.tracks.len() <= self.workspaces.len());
        self.refresh_pointer_surface(0);
        self.request_repaint();
        self.schedule_motion_tick();
    }
    /// One refresh-paced timer for all timed workspace motion; no timer during finger tracking or idle.
    pub(crate) fn schedule_motion_tick(&mut self) {
        if self.workspace_motion.timer.is_some() || !self.timed_workspace_motion() {
            return;
        }
        let refresh = self
            .space
            .outputs()
            .next()
            .and_then(|output| output.current_mode())
            .map(|mode| mode.refresh)
            .filter(|refresh| *refresh > 0)
            .unwrap_or(60_000) as u64;
        let interval = Duration::from_nanos(1_000_000_000_000 / refresh);
        match self
            .loop_handle
            .insert_source(Timer::from_duration(interval), move |_, _, state| {
                let now = Instant::now();
                let changed = state.workspace_motion.advance(state.active_workspace, now);
                let swipe_changed = state.advance_swipe_settle(now);
                if changed || swipe_changed {
                    state.request_repaint();
                }
                if state.timed_workspace_motion() {
                    TimeoutAction::ToDuration(interval)
                } else {
                    state.workspace_motion.timer = None;
                    state.trim_dynamic_workspaces();
                    state.refresh_pointer_surface(0);
                    TimeoutAction::Drop
                }
            }) {
            Ok(timer) => self.workspace_motion.timer = Some(timer),
            Err(error) => {
                tracing::warn!(%error, "could not schedule workspace motion");
                self.workspace_motion.tracks.clear();
                self.stop_swipe_visual();
                self.refresh_pointer_surface(0);
                self.request_repaint();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn switches_retarget_from_current_positions_and_remain_bounded() {
        let now = Instant::now();
        let mut motion = WorkspaceMotion::default();
        motion.start(0, 1, Direction::Next, 1280, now);
        let midway = now + SETTLE / 2;
        let before = motion.scenes(midway);
        motion.start(1, 2, Direction::Next, 1280, midway);
        for (workspace, offset) in before {
            assert_eq!(
                motion
                    .scenes(midway)
                    .iter()
                    .find(|(id, _)| *id == workspace)
                    .unwrap()
                    .1,
                offset
            );
        }
        assert_eq!(motion.scenes(midway)[0], (2, 1440.0));
        let before = motion.scenes(midway + SETTLE / 4);
        motion.start(2, 1, Direction::Previous, 1280, midway + SETTLE / 4);
        for (workspace, offset) in before {
            assert_eq!(
                motion
                    .scenes(midway + SETTLE / 4)
                    .iter()
                    .find(|(id, _)| *id == workspace)
                    .unwrap()
                    .1,
                offset
            );
        }
        for index in 0..1000 {
            motion.start(index % 10, (index + 1) % 10, Direction::Next, 1280, midway);
            assert!(motion.tracks.len() <= 10);
        }
        motion.advance(0, now + Duration::from_secs(2));
        assert!(!motion.active());
    }
    #[test]
    fn wrap_direction_uses_navigation_intent_and_same_goal_does_not_restart() {
        let now = Instant::now();
        let mut motion = WorkspaceMotion::default();
        motion.start(9, 0, Direction::Next, 1280, now);
        assert_eq!(motion.scenes(now), vec![(0, 1280.0), (9, 0.0)]);
        motion.advance(0, now + SETTLE);
        assert!(!motion.active());
        motion.start(0, 9, Direction::Previous, 1280, now);
        assert_eq!(motion.scenes(now)[0], (9, -1280.0));
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use smithay::reexports::{calloop::EventLoop, wayland_server::Display};

    #[test]
    #[ignore = "requires private Unix sockets and an isolated XDG_RUNTIME_DIR"]
    fn shared_driver_finishes_and_cancels_without_leaving_timers() {
        let mut event_loop = EventLoop::try_new().unwrap();
        let mut state = Villain::new(
            &mut event_loop,
            Display::new().unwrap(),
            crate::config::RuntimeConfig::load().unwrap(),
        );
        state.output_size = (1280, 800).into();
        state.switch_workspace_immediate(1);
        state
            .workspace_motion
            .start(0, 1, Direction::Next, 1280, Instant::now());
        state.schedule_motion_tick();
        let timer = state.workspace_motion.timer;
        state.schedule_motion_tick();
        assert_eq!(
            state.workspace_motion.timer, timer,
            "coalesce timer requests"
        );
        assert!(state.workspace_scene_visible(0));
        assert!(state.workspace_scene_visible(1));
        let deadline = Instant::now() + Duration::from_secs(1);
        while state.timed_workspace_motion() {
            assert!(Instant::now() < deadline);
            event_loop
                .dispatch(Duration::from_millis(20), &mut state)
                .unwrap();
        }
        assert!(state.workspace_motion.timer.is_none());
        assert!(!state.workspace_scene_visible(0));
        assert_eq!(state.active_workspace, 1);
        assert_eq!(state.last_workspace, Some(0));

        for interruption in 0..4 {
            state
                .workspace_motion
                .start(0, 1, Direction::Next, 1280, Instant::now());
            state.schedule_motion_tick();
            match interruption {
                0 => {
                    state.host_focused = false;
                    state.maintain_swipe();
                    state.host_focused = true;
                }
                1 => {
                    state
                        .dispatch(crate::dispatch::Dispatch::SetOverviewVisible(true))
                        .unwrap();
                }
                2 => {
                    state.finish_workspace_motion();
                }
                _ => {
                    state.cancel_workspace_motion();
                }
            }
            assert!(!state.timed_workspace_motion());
            assert!(state.workspace_motion.timer.is_none());
            state
                .dispatch(crate::dispatch::Dispatch::SetOverviewVisible(false))
                .unwrap();
        }
        state
            .workspace_motion
            .start(0, 1, Direction::Next, 1280, Instant::now());
        state.schedule_motion_tick();
        state.handle_compositor_button(0x110, smithay::backend::input::ButtonState::Pressed, 1);
        assert!(!state.timed_workspace_motion());
        assert!(state.workspace_motion.timer.is_none());
        assert_eq!(state.active_workspace, 1);
        state.release_pointer_buttons();
    }
}
