//! The boot visual sequence (§17).
//!
//!   fade in → owl video → owl fade out → MĨȚǑŠ fade in → hold →
//!   fade out → handoff
//!
//! Two hard rules from the design document are enforced here:
//!   1. BOOT ANIMATION ≠ SYSTEM READINESS (§21): the wordmark hold extends
//!      indefinitely until the system is ready (or the readiness timeout
//!      fires, because a stuck animation must never wedge the boot).
//!   2. No arbitrary sleeps: every phase change is event- or time-driven,
//!      and a stalled video is force-exited after VIDEO_STALL_CAP_S.

use super::transition::{Easing, Transition};
use crate::config::AnimationSection;

/// If the system is ready to continue and the video has still not finished
/// within this window, force the transition (watchdog backstop).
const VIDEO_STALL_CAP_S: f64 = 20.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    FadeIn,
    Video,
    OwlFadeOut,
    WordmarkIn,
    WordmarkHold,
    WordmarkOut,
    WaitReady,
    Complete,
}

pub struct SplashTimeline {
    fade_in: f64,
    fade_out: f64,
    wm_in: f64,
    wm_out: f64,
    wm_hold: f64,
    min_duration: f64,
    has_video: bool,
    has_wordmark: bool,
    fast_when_ready: bool,
    phase: Phase,
    phase_start: f64,
    last_elapsed: f64,
    forced_video_exit: bool,
}

fn s(ms: u32) -> f64 { ms as f64 / 1000.0 }

impl SplashTimeline {
    pub fn new(a: &AnimationSection, owl_absent: bool, wordmark_enabled: bool) -> Self {
        let mut t = SplashTimeline {
            fade_in: s(a.fade_in_ms),
            fade_out: s(a.fade_out_ms),
            wm_in: s(a.wordmark_in_ms),
            wm_out: s(a.wordmark_out_ms),
            wm_hold: s(a.wordmark_hold_ms),
            min_duration: s(a.minimum_duration_ms),
            has_video: !owl_absent,
            has_wordmark: wordmark_enabled,
            fast_when_ready: a.fast_when_ready,
            phase: Phase::FadeIn,
            phase_start: 0.0,
            last_elapsed: 0.0,
            forced_video_exit: false,
        };
        if !t.has_video {
            // Nothing to fade in: go straight to the wordmark (or readiness wait).
            t.phase = if t.has_wordmark { Phase::WordmarkIn } else { Phase::WaitReady };
        }
        t
    }

    pub fn phase(&self) -> Phase { self.phase }

    /// Advance to `elapsed_s` (seconds since the visual loop started).
    /// `video_done` comes from the splash; `ready`/`timed_out` from the
    /// readiness monitor. Returns the current phase. Multiple completed
    /// zero-length phases may be crossed in one call.
    pub fn update(&mut self, elapsed_s: f64, video_done: bool, ready: bool, timed_out: bool) -> Phase {
        self.last_elapsed = elapsed_s;
        let go = ready || timed_out;

        for _ in 0..8 {
            let t = elapsed_s - self.phase_start;
            let next = match self.phase {
                Phase::FadeIn => {
                    (t >= self.fade_in).then_some(if self.has_video { Phase::Video } else { Phase::WaitReady })
                }
                Phase::Video => {
                    let min_ok = elapsed_s >= self.min_duration;
                    if video_done && min_ok {
                        Some(Phase::OwlFadeOut)
                    } else if go && t >= VIDEO_STALL_CAP_S && !self.forced_video_exit {
                        self.forced_video_exit = true;
                        log::warn!("owl video stalled {VIDEO_STALL_CAP_S:.0}s; forcing transition");
                        Some(Phase::OwlFadeOut)
                    } else {
                        None // video still playing: it drives its own pace
                    }
                }
                Phase::OwlFadeOut => {
                    (t >= self.fade_out).then_some(if self.has_wordmark { Phase::WordmarkIn } else { Phase::WaitReady })
                }
                Phase::WordmarkIn => (t >= self.wm_in).then_some(Phase::WordmarkHold),
                Phase::WordmarkHold => {
                    // §22: hold extends until the system is ready. If it is
                    // already ready, exit fast (fast_when_ready) or after the
                    // base hold for a consistent visual rhythm.
                    let held_long_enough = t >= self.wm_hold;
                    (go && (held_long_enough || self.fast_when_ready)).then_some(Phase::WordmarkOut)
                }
                Phase::WordmarkOut => (t >= self.wm_out).then_some(Phase::Complete),
                Phase::WaitReady => go.then_some(Phase::Complete),
                Phase::Complete => None,
            };
            match next {
                Some(p) => { self.phase = p; self.phase_start = elapsed_s; }
                None => break,
            }
        }
        self.phase
    }

    fn span_progress(&self, duration: f64) -> f32 {
        Transition::new(self.phase_start, duration, Easing::Smoothstep)
            .progress_at(self.last_elapsed)
    }

    pub fn owl_alpha(&self) -> f32 {
        match self.phase {
            Phase::FadeIn => self.span_progress(self.fade_in),
            Phase::Video => 1.0,
            Phase::OwlFadeOut => 1.0 - self.span_progress(self.fade_out),
            _ => 0.0,
        }
    }

    pub fn wordmark_alpha(&self) -> f32 {
        match self.phase {
            Phase::WordmarkIn => self.span_progress(self.wm_in),
            Phase::WordmarkHold => 1.0,
            Phase::WordmarkOut => 1.0 - self.span_progress(self.wm_out),
            _ => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anim() -> AnimationSection {
        AnimationSection {
            fade_in_ms: 400, fade_out_ms: 500, wordmark_in_ms: 500,
            wordmark_out_ms: 500, wordmark_hold_ms: 600,
            minimum_duration_ms: 1800, fast_when_ready: true,
        }
    }

    #[test]
    fn no_owl_completes_when_ready() {
        let mut tl = SplashTimeline::new(&anim(), true, true);
        assert_eq!(tl.phase(), Phase::WordmarkIn);
        assert_eq!(tl.update(10.0, true, false, false), Phase::WordmarkHold);
        // Not ready yet: the hold extends indefinitely.
        assert_eq!(tl.update(60.0, true, false, false), Phase::WordmarkHold);
        assert_eq!(tl.update(60.5, true, true, false), Phase::WordmarkOut);
        assert_eq!(tl.update(61.5, true, true, false), Phase::Complete);
    }

    #[test]
    fn video_gates_transition() {
        let mut tl = SplashTimeline::new(&anim(), false, true);
        assert_eq!(tl.update(0.5, false, false, false), Phase::FadeIn); // still fading
        assert_eq!(tl.update(0.6, false, false, false), Phase::Video);
        // Video done but minimum duration not reached: hold the last frame.
        assert_eq!(tl.update(1.0, true, false, false), Phase::Video);
        assert_eq!(tl.update(1.9, true, false, false), Phase::OwlFadeOut);
    }

    #[test]
    fn readiness_timeout_releases_hold() {
        let mut tl = SplashTimeline::new(&anim(), true, false);
        assert_eq!(tl.phase(), Phase::WaitReady);
        assert_eq!(tl.update(5.0, true, false, false), Phase::WaitReady);
        assert_eq!(tl.update(5.0, true, false, true), Phase::Complete);
    }

    #[test]
    fn stalled_video_is_force_exited() {
        let mut tl = SplashTimeline::new(&anim(), false, true);
        tl.update(0.6, false, false, false);
        assert_eq!(tl.update(25.0, false, true, false), Phase::OwlFadeOut);
    }
}