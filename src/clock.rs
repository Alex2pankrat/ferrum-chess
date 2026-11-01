//! Monotonic local two-player clock. Time is charged before each action.
use cozy_chess::Color;
use std::{
    fmt,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Off,
    ThreePlusTwo,
    FivePlusThree,
    Ten,
    FifteenPlusTen,
}
impl fmt::Display for Preset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Off => "Off",
            Self::ThreePlusTwo => "3 + 2",
            Self::FivePlusThree => "5 + 3",
            Self::Ten => "10 + 0",
            Self::FifteenPlusTen => "15 + 10",
        })
    }
}
impl Preset {
    fn times(self) -> (Duration, Duration) {
        let (minutes, increment) = match self {
            Self::Off => (0, 0),
            Self::ThreePlusTwo => (3, 2),
            Self::FivePlusThree => (5, 3),
            Self::Ten => (10, 0),
            Self::FifteenPlusTen => (15, 10),
        };
        (
            Duration::from_secs(minutes * 60),
            Duration::from_secs(increment),
        )
    }
}
#[derive(Debug, Clone)]
pub struct Clock {
    preset: Preset,
    white: Duration,
    black: Duration,
    running: Option<(Color, Instant)>,
}
impl Default for Clock {
    fn default() -> Self {
        Self::new(Preset::Off)
    }
}
impl Clock {
    pub fn new(preset: Preset) -> Self {
        let (base, _) = preset.times();
        Self {
            preset,
            white: base,
            black: base,
            running: None,
        }
    }
    pub fn preset(&self) -> Preset {
        self.preset
    }
    pub fn reset(&mut self) {
        *self = Self::new(self.preset);
    }
    pub fn enabled(&self) -> bool {
        self.preset != Preset::Off
    }
    pub fn running_side(&self) -> Option<Color> {
        self.running.map(|(side, _)| side)
    }
    pub fn remaining(&self, side: Color) -> Duration {
        if side == Color::White {
            self.white
        } else {
            self.black
        }
    }
    fn remaining_mut(&mut self, side: Color) -> &mut Duration {
        if side == Color::White {
            &mut self.white
        } else {
            &mut self.black
        }
    }
    /// Charge elapsed monotonic time; return the side that has just flagged.
    pub fn sync(&mut self, now: Instant) -> Option<Color> {
        let (side, since) = self.running?;
        let remaining = self.remaining_mut(side);
        *remaining = remaining.saturating_sub(now.saturating_duration_since(since));
        if remaining.is_zero() {
            self.running = None;
            Some(side)
        } else {
            self.running = Some((side, now));
            None
        }
    }
    pub fn start(&mut self, side: Color, now: Instant) -> bool {
        if !self.enabled() || self.running.is_some() || self.remaining(side).is_zero() {
            return false;
        }
        self.running = Some((side, now));
        true
    }
    pub fn pause(&mut self) {
        self.running = None;
    }
    /// Called only after a legal move; caller syncs immediately before handling it.
    pub fn completed_move(&mut self, side: Color, now: Instant) {
        if self.running_side() != Some(side) {
            return;
        }
        let increment = self.preset.times().1;
        *self.remaining_mut(side) += increment;
        self.running = Some((!side, now));
    }
    pub fn formatted(&self, side: Color) -> String {
        let seconds = self.remaining(side).as_millis().div_ceil(1000);
        format!("{:02}:{:02}", seconds / 60, seconds % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn charges_correct_side_applies_increment_and_pauses() {
        let start = Instant::now();
        let mut clock = Clock::new(Preset::ThreePlusTwo);
        assert!(clock.start(Color::White, start));
        assert_eq!(clock.sync(start + Duration::from_secs(5)), None);
        clock.completed_move(Color::White, start + Duration::from_secs(5));
        assert_eq!(clock.remaining(Color::White), Duration::from_secs(177));
        assert_eq!(clock.running_side(), Some(Color::Black));
        clock.sync(start + Duration::from_secs(8));
        clock.pause();
        assert_eq!(clock.sync(start + Duration::from_secs(50)), None);
        assert_eq!(clock.remaining(Color::Black), Duration::from_secs(177));
        assert!(clock.start(Color::Black, start + Duration::from_secs(50)));
        assert_eq!(
            clock.sync(start + Duration::from_secs(227)),
            Some(Color::Black)
        );
        assert!(!clock.start(Color::Black, start + Duration::from_secs(228)));
    }
    #[test]
    fn off_and_reset_are_explicit() {
        let mut clock = Clock::default();
        assert!(!clock.start(Color::White, Instant::now()));
        clock = Clock::new(Preset::Ten);
        let now = Instant::now();
        clock.start(Color::Black, now);
        clock.sync(now + Duration::from_secs(1));
        clock.reset();
        assert_eq!(clock.remaining(Color::Black), Duration::from_secs(600));
        assert_eq!(clock.running_side(), None);
    }
}
