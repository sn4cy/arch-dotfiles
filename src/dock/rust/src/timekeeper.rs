//! Monotonic timer and stopwatch. Independent of GTK and wall-clock changes.
use std::time::{Duration, Instant};
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Timer,
    Stopwatch,
}
#[derive(Debug)]
pub struct Timekeeper {
    pub mode: Mode,
    pub duration: Duration,
    remaining: Duration,
    elapsed: Duration,
    started: Option<Instant>,
    pub finished: bool,
}
impl Default for Timekeeper {
    fn default() -> Self {
        Self {
            mode: Mode::Timer,
            duration: Duration::from_secs(25 * 60),
            remaining: Duration::from_secs(25 * 60),
            elapsed: Duration::ZERO,
            started: None,
            finished: false,
        }
    }
}
impl Timekeeper {
    pub fn running(&self) -> bool {
        self.started.is_some()
    }
    pub fn value(&self, now: Instant) -> Duration {
        let delta = self
            .started
            .map_or(Duration::ZERO, |s| now.saturating_duration_since(s));
        match self.mode {
            Mode::Timer => self.remaining.saturating_sub(delta),
            Mode::Stopwatch => self.elapsed.saturating_add(delta),
        }
    }
    pub fn pause(&mut self, now: Instant) {
        let value = self.value(now);
        match self.mode {
            Mode::Timer => self.remaining = value,
            Mode::Stopwatch => self.elapsed = value,
        }
        self.started = None;
    }
    pub fn toggle(&mut self, now: Instant) {
        if self.running() {
            self.pause(now);
        } else {
            if self.mode == Mode::Timer && self.remaining.is_zero() {
                self.remaining = self.duration;
            }
            self.finished = false;
            self.started = Some(now);
        }
    }
    pub fn set_mode(&mut self, mode: Mode, now: Instant) {
        if self.mode != mode {
            self.pause(now);
            self.mode = mode;
            self.finished = false;
        }
    }
    pub fn set_duration(&mut self, seconds: u64, now: Instant) {
        self.pause(now);
        self.mode = Mode::Timer;
        self.duration = Duration::from_secs(seconds.max(1));
        self.remaining = self.duration;
        self.finished = false;
    }
    pub fn reset(&mut self) {
        self.started = None;
        self.finished = false;
        match self.mode {
            Mode::Timer => self.remaining = self.duration,
            Mode::Stopwatch => self.elapsed = Duration::ZERO,
        }
    }
    pub fn tick(&mut self, now: Instant) -> bool {
        if self.mode == Mode::Timer && self.running() && self.value(now).is_zero() {
            self.remaining = Duration::ZERO;
            self.started = None;
            self.finished = true;
            true
        } else {
            false
        }
    }
    pub fn label(&self, now: Instant) -> String {
        let d = self.value(now);
        let seconds = if self.mode == Mode::Timer {
            d.as_secs() + u64::from(d.subsec_nanos() > 0)
        } else {
            d.as_secs()
        };
        let base = if seconds >= 3600 {
            format!(
                "{}:{:02}:{:02}",
                seconds / 3600,
                seconds / 60 % 60,
                seconds % 60
            )
        } else {
            format!("{:02}:{:02}", seconds / 60, seconds % 60)
        };
        if self.mode == Mode::Stopwatch {
            format!("{base}.{}", d.subsec_millis() / 100)
        } else {
            base
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn countdown_pause_resume_and_single_alarm() {
        let t = Instant::now();
        let mut c = Timekeeper::default();
        c.set_duration(10, t);
        c.toggle(t);
        assert_eq!(c.label(t + Duration::from_millis(1500)), "00:09");
        c.pause(t + Duration::from_secs(3));
        assert_eq!(c.value(t + Duration::from_secs(20)), Duration::from_secs(7));
        c.toggle(t + Duration::from_secs(30));
        assert!(!c.tick(t + Duration::from_secs(36)));
        assert!(c.tick(t + Duration::from_secs(37)));
        assert!(!c.tick(t + Duration::from_secs(38)));
        assert!(c.finished);
        c.toggle(t + Duration::from_secs(40));
        assert_eq!(
            c.value(t + Duration::from_secs(40)),
            Duration::from_secs(10)
        );
    }
    #[test]
    fn stopwatch_keeps_paused_time_and_resets() {
        let t = Instant::now();
        let mut c = Timekeeper::default();
        c.set_mode(Mode::Stopwatch, t);
        c.toggle(t);
        c.pause(t + Duration::from_millis(1234));
        assert_eq!(c.label(t + Duration::from_secs(50)), "00:01.2");
        c.toggle(t + Duration::from_secs(60));
        assert_eq!(c.label(t + Duration::from_millis(62766)), "00:04.0");
        c.reset();
        assert_eq!(c.label(t), "00:00.0");
    }
    #[test]
    fn switching_modes_pauses_and_preserves_each_value() {
        let t = Instant::now();
        let mut c = Timekeeper::default();
        c.set_duration(60, t);
        c.toggle(t);
        c.set_mode(Mode::Stopwatch, t + Duration::from_secs(10));
        assert!(!c.running());
        c.toggle(t + Duration::from_secs(20));
        c.set_mode(Mode::Timer, t + Duration::from_secs(25));
        assert_eq!(
            c.value(t + Duration::from_secs(100)),
            Duration::from_secs(50)
        );
        c.set_mode(Mode::Stopwatch, t + Duration::from_secs(101));
        assert_eq!(c.label(t), "00:05.0");
    }
}
