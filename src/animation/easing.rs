//! Easing curves. Pure functions, no state.

pub fn clamp01(t: f32) -> f32 { t.clamp(0.0, 1.0) }

pub fn linear(t: f32) -> f32 { clamp01(t) }

pub fn smoothstep(t: f32) -> f32 {
    let t = clamp01(t);
    t * t * (3.0 - 2.0 * t)
}

pub fn ease_out_cubic(t: f32) -> f32 {
    let t = clamp01(t);
    1.0 - (1.0 - t).powi(3)
}

pub fn ease_in_out_cubic(t: f32) -> f32 {
    let t = clamp01(t);
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoints_and_monotonicity() {
        for f in [linear, smoothstep, ease_out_cubic, ease_in_out_cubic] {
            assert_eq!(f(-1.0), 0.0);
            assert_eq!(f(0.0), 0.0);
            assert_eq!(f(1.0), 1.0);
            assert_eq!(f(2.0), 1.0);
            assert!(f(0.25) <= f(0.75));
        }
    }
}