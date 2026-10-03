//! Value ramps (fade in / fade out) used by the timeline for all alphas.

use super::transition::{Easing, Transition};

pub struct Fade {
    from: f32,
    to: f32,
    span: Transition,
}

impl Fade {
    pub fn new(from: f32, to: f32, duration_ms: u32) -> Self {
        Self {
            from,
            to,
            span: Transition::new(0.0, duration_ms as f64 / 1000.0, Easing::Smoothstep),
        }
    }

    /// Value at `t_ms` milliseconds into the fade.
    pub fn value_at(&self, t_ms: f64) -> f32 {
        self.from + (self.to - self.from) * self.span.progress_at(t_ms / 1000.0)
    }
}

pub fn fade_in(t_ms: f64, duration_ms: u32) -> f32 { Fade::new(0.0, 1.0, duration_ms).value_at(t_ms) }
pub fn fade_out(t_ms: f64, duration_ms: u32) -> f32 { Fade::new(1.0, 0.0, duration_ms).value_at(t_ms) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ramps() {
        assert_eq!(fade_in(0.0, 400), 0.0);
        assert_eq!(fade_in(400.0, 400), 1.0);
        assert_eq!(fade_out(0.0, 400), 1.0);
        assert_eq!(fade_out(400.0, 400), 0.0);
        assert!((fade_in(200.0, 400) - 0.5).abs() < 1e-3); // smoothstep is symmetric
    }
}