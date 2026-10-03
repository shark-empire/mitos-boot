//! A timed span that maps time → eased progress.

use super::easing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Easing { Linear, Smoothstep, EaseOutCubic, EaseInOutCubic }

impl Easing {
    pub fn apply(self, t: f32) -> f32 {
        match self {
            Self::Linear => easing::linear(t),
            Self::Smoothstep => easing::smoothstep(t),
            Self::EaseOutCubic => easing::ease_out_cubic(t),
            Self::EaseInOutCubic => easing::ease_in_out_cubic(t),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Transition {
    pub start_s: f64,
    pub duration_s: f64,
    pub easing: Easing,
}

impl Transition {
    pub fn new(start_s: f64, duration_s: f64, easing: Easing) -> Self {
        Self { start_s, duration_s, easing }
    }

    /// Progress in 0..1 at absolute time `t` (seconds). Zero-length spans
    /// snap to done at their start time.
    pub fn progress_at(&self, t: f64) -> f32 {
        if self.duration_s <= 0.0 {
            return if t >= self.start_s { 1.0 } else { 0.0 };
        }
        self.easing.apply(((t - self.start_s) / self.duration_s) as f32)
    }
}