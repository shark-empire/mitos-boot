//! Software frame pacing for backends without vsync events (fbdev).

use std::time::{Duration, Instant};

pub struct Ticker { interval: Duration, next: Instant }

impl Ticker {
    pub fn new(fps: f64) -> Self {
        let fps = if fps.is_finite() && fps >= 1.0 { fps.min(240.0) } else { 60.0 };
        let interval = Duration::from_secs_f64(1.0 / fps);
        Self { interval, next: Instant::now() + interval }
    }

    /// Sleep until the next tick. If we are already behind (e.g. after a slow
    /// decode), resynchronize instead of bursting through missed ticks.
    pub fn wait(&mut self) {
        let now = Instant::now();
        if self.next > now { std::thread::sleep(self.next - now); }
        self.next += self.interval;
        let now = Instant::now();
        if self.next <= now { self.next = now + self.interval; }
    }
}