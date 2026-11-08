//! Pace client callbacks independently of whether a rendered frame has damage.

use std::time::Duration;

use smithay::{
    output::Output,
    reexports::calloop::timer::{TimeoutAction, Timer},
};

use crate::state::Villain;

#[derive(Default)]
pub struct FrameClock {
    last_sent: Option<Duration>,
    pending: bool,
}

impl FrameClock {
    fn request(&mut self, now: Duration, refresh: Option<i32>) -> Option<Duration> {
        if self.pending {
            return None;
        }
        self.pending = true;
        let refresh = refresh.filter(|refresh| *refresh > 0).unwrap_or(60_000) as u64;
        // Wayland output refresh rates are in millihertz.
        let interval = Duration::from_nanos(1_000_000_000_000 / refresh);
        let due = self.last_sent.unwrap_or(now) + interval;
        Some(due.saturating_sub(now))
    }

    fn finish(&mut self, now: Duration) {
        self.pending = false;
        self.last_sent = Some(now);
    }
}

impl Villain {
    /// A frame callback is permission to draw again, not a presentation receipt.
    /// A successful render with no damage still needs to wake waiting clients.
    /// One demand-driven timer bounds callback-only commit loops to output refresh.
    pub fn schedule_frame_callbacks(&mut self, output: &Output) {
        let Some(delay) = self.frame_clock.request(
            self.start_time.elapsed(),
            output.current_mode().map(|mode| mode.refresh),
        ) else {
            return;
        };
        let output = output.clone();
        let result =
            self.loop_handle
                .insert_source(Timer::from_duration(delay), move |_, _, state| {
                    if state.tty.as_ref().is_some_and(|tty| !tty.is_active()) {
                        state.frame_clock.pending = false;
                        return TimeoutAction::Drop;
                    }
                    let now = state.start_time.elapsed();
                    state.frame_clock.finish(now);
                    // Resolve visibility when the timer fires: a workspace may have
                    // changed since the render that scheduled this callback.
                    for window in state.space.elements() {
                        Villain::send_window_frame(window, &output, now);
                    }
                    for index in 0..state.workspaces.len() {
                        if !state.workspace_scene_visible(index)
                            && !state
                                .overview_panes
                                .iter()
                                .any(|pane| pane.workspace.0 as usize == index + 1)
                        {
                            continue;
                        }
                        if index != state.active_workspace
                            && let Some(scene) = state.workspace_preview_scene(index)
                        {
                            for (window, _) in scene.windows {
                                Villain::send_window_frame(&window, &output, now);
                            }
                        }
                    }
                    for entry in &state.shell_surfaces {
                        if entry.mapped && entry.output == output {
                            entry
                                .layer
                                .send_frame(&output, now, None, |_, _| Some(output.clone()));
                        }
                    }
                    state.cursor.send_frame(&output, now);
                    TimeoutAction::Drop
                });
        if let Err(error) = result {
            self.frame_clock.pending = false;
            tracing::warn!(%error, "could not schedule client frame callbacks");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_only_commits_are_paced_and_coalesced() {
        let mut clock = FrameClock::default();
        let interval = Duration::from_nanos(16_666_666);
        assert_eq!(clock.request(Duration::ZERO, Some(60_000)), Some(interval));
        assert_eq!(clock.request(Duration::from_millis(1), Some(60_000)), None);
        clock.finish(interval);
        assert_eq!(clock.request(interval, Some(60_000)), Some(interval));
        clock.finish(interval * 2);
        // After idle, a requested callback can be delivered immediately.
        assert_eq!(
            clock.request(Duration::from_secs(1), Some(60_000)),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn uses_output_refresh_and_handles_missing_or_invalid_modes() {
        for refresh in [None, Some(0), Some(-1), Some(60_000)] {
            assert_eq!(
                FrameClock::default().request(Duration::ZERO, refresh),
                Some(Duration::from_nanos(16_666_666))
            );
        }
        assert_eq!(
            FrameClock::default().request(Duration::ZERO, Some(144_000)),
            Some(Duration::from_nanos(6_944_444))
        );
    }
}
