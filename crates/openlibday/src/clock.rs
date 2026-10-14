//! Time source for animations, timers and the AI's turn: the wall clock, or
//! a virtual clock that only moves when a script advances it, so scripted
//! runs (and the frames they record) are reproducible.

use std::time::{Duration, Instant};

pub struct Clock {
    origin: Instant,
    /// Virtual time, when not following the wall clock.
    manual: Option<Duration>,
}

impl Clock {
    pub fn real() -> Self {
        Self {
            origin: Instant::now(),
            manual: None,
        }
    }

    pub fn manual() -> Self {
        Self {
            origin: Instant::now(),
            manual: Some(Duration::ZERO),
        }
    }

    /// Time since the clock started.
    pub fn now(&self) -> Duration {
        self.manual.unwrap_or_else(|| self.origin.elapsed())
    }

    /// Time since `then` (a value of `now`).
    pub fn since(&self, then: Duration) -> Duration {
        self.now().saturating_sub(then)
    }

    /// Moves a virtual clock forward; the wall clock ignores it.
    pub fn advance(&mut self, by: Duration) {
        if let Some(t) = &mut self.manual {
            *t += by;
        }
    }

    /// Switches a virtual clock to the wall clock, carrying on from its time.
    pub fn go_live(&mut self) {
        if let Some(t) = self.manual.take() {
            self.origin = Instant::now().checked_sub(t).unwrap_or_else(Instant::now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_moves_only_when_advanced() {
        let mut clock = Clock::manual();
        assert_eq!(clock.now(), Duration::ZERO);
        clock.advance(Duration::from_millis(250));
        assert_eq!(clock.now(), Duration::from_millis(250));
        assert_eq!(clock.since(Duration::from_millis(100)).as_millis(), 150);
        assert_eq!(clock.since(Duration::from_secs(1)), Duration::ZERO);
    }
}
