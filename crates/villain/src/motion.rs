//! Pure elapsed-time motion. Callers own clocks, scheduling, damage and lifetimes.
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Sample {
    pub value: f64,
    pub finished: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Tween {
    from: f64,
    goal: f64,
    started: Instant,
    duration: Duration,
}
impl Tween {
    pub fn new(from: f64, goal: f64, started: Instant, duration: Duration) -> Self {
        assert!(from.is_finite() && goal.is_finite());
        Self {
            from,
            goal,
            started,
            duration,
        }
    }
    pub fn sample(&self, now: Instant) -> Sample {
        let elapsed = now.saturating_duration_since(self.started);
        if self.from == self.goal || elapsed >= self.duration {
            return Sample {
                value: self.goal,
                finished: true,
            };
        }
        let progress = elapsed.as_secs_f64() / self.duration.as_secs_f64();
        let eased = 1.0 - (1.0 - progress).powi(3);
        Sample {
            value: self.from + (self.goal - self.from) * eased,
            finished: false,
        }
    }
    /// Keep position continuous. Cubic easing restarts; velocity is not preserved.
    pub fn retarget(&mut self, goal: f64, now: Instant) {
        assert!(goal.is_finite());
        if goal == self.goal {
            return;
        }
        self.from = self.sample(now).value;
        self.goal = goal;
        self.started = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn samples_depend_on_time_not_frame_count() {
        let start = Instant::now();
        let tween = Tween::new(0.0, 100.0, start, Duration::from_millis(160));
        assert_eq!(tween.sample(start).value, 0.0);
        let halfway = start + Duration::from_millis(80);
        assert_eq!(tween.sample(halfway).value, 87.5);
        for millis in 0..80 {
            tween.sample(start + Duration::from_millis(millis));
        }
        assert_eq!(tween.sample(halfway).value, 87.5);
        let end = tween.sample(start + Duration::from_secs(30));
        assert!(end.finished);
        assert_eq!(end.value, 100.0);
        assert_eq!(tween.sample(start - Duration::from_secs(1)).value, 0.0);
    }
    #[test]
    fn retargets_and_reversals_preserve_position_and_repeated_goals_preserve_time() {
        let start = Instant::now();
        let mut tween = Tween::new(0.0, 100.0, start, Duration::from_millis(160));
        let now = start + Duration::from_millis(40);
        let current = tween.sample(now).value;
        tween.retarget(-100.0, now);
        assert_eq!(tween.sample(now).value, current);
        tween.retarget(-100.0, now + Duration::from_millis(80));
        assert!(tween.sample(now + Duration::from_millis(160)).finished);
    }
    #[test]
    fn zero_duration_and_stationary_motion_finish_exactly() {
        let now = Instant::now();
        assert_eq!(
            Tween::new(3.0, 7.0, now, Duration::ZERO).sample(now).value,
            7.0
        );
        assert!(
            Tween::new(3.0, 3.0, now, Duration::from_secs(1))
                .sample(now)
                .finished
        );
    }
}
