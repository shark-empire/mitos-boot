//! Playback control (§10). The player never touches DRM — it hands frames to
//! the renderer and answers "finished?". Pacing is by presentation time when
//! the container provides timestamps, else by the estimated frame rate.

use super::decoder::{self, VideoDecoder, VideoInfo};
use super::frame::VideoFrame;
use crate::config::VideoSection;
use crate::error::BootError;
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerState { Idle, Playing, Paused, Finished, Failed }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerUpdate { Holding, Advanced, Finished, Failed }

/// Pause-aware wall clock.
struct Clock { accumulated: f64, resume_at: Instant, running: bool }
impl Clock {
    fn new() -> Self { Self { accumulated: 0.0, resume_at: Instant::now(), running: false } }
    fn start(&mut self) {
        if !self.running { self.running = true; self.resume_at = Instant::now(); }
    }
    fn pause(&mut self) {
        if self.running {
            self.accumulated += self.resume_at.elapsed().as_secs_f64();
            self.running = false;
        }
    }
    fn elapsed(&self) -> f64 {
        self.accumulated + if self.running { self.resume_at.elapsed().as_secs_f64() } else { 0.0 }
    }
}

pub struct VideoPlayer {
    decoder: Box<dyn VideoDecoder>,
    state: PlayerState,
    current: Option<VideoFrame>,
    next: Option<VideoFrame>, // one-frame lookahead for pts-paced advance
    clock: Clock,
    fps: f64,
    frames_shown: u64,
    decode_budget: Duration,
    warned_slow: bool,
}

impl VideoPlayer {
    pub fn open(path: &Path, cfg: &VideoSection) -> Result<Self, BootError> {
        let decoder = decoder::open(path, cfg)?;
        let fps = decoder.info().estimated_fps.clamp(1.0, 240.0);
        let mut p = VideoPlayer {
            decoder, state: PlayerState::Idle, current: None, next: None,
            clock: Clock::new(), fps, frames_shown: 0,
            decode_budget: Duration::from_millis(cfg.max_decode_per_update_ms as u64),
            warned_slow: false,
        };
        p.current = p.decoder.next_frame();
        p.next = p.decoder.next_frame();
        if p.current.is_none() { p.state = PlayerState::Finished; }
        Ok(p)
    }

    pub fn info(&self) -> &VideoInfo { self.decoder.info() }
    pub fn state(&self) -> PlayerState { self.state }
    pub fn finished(&self) -> bool { matches!(self.state, PlayerState::Finished | PlayerState::Failed) }
    pub fn frame(&self) -> Option<&VideoFrame> { self.current.as_ref() }
    pub fn frames_shown(&self) -> u64 { self.frames_shown }

    pub fn play(&mut self) {
        if self.state == PlayerState::Idle || self.state == PlayerState::Paused {
            self.state = PlayerState::Playing;
        }
        self.clock.start();
    }
    #[allow(dead_code)] // part of the §10 API; splash uses play/update only
    pub fn pause(&mut self) {
        if self.state == PlayerState::Playing { self.state = PlayerState::Paused; }
        self.clock.pause();
    }
    #[allow(dead_code)]
    pub fn resume(&mut self) { self.play(); }
    #[allow(dead_code)]
    pub fn stop(&mut self) {
        self.state = PlayerState::Finished;
        self.current = None;
        self.next = None;
    }

    /// Advance playback to wall-clock "now". Decodes at most one frame per
    /// call, bounding decode cost per display cycle (§32) and preventing the
    /// decoder from outrunning the display.
    pub fn update(&mut self, now: Instant) -> PlayerUpdate {
        let _ = now; // the clock is self-contained; parameter kept for API clarity
        match self.state {
            PlayerState::Finished => return PlayerUpdate::Finished,
            PlayerState::Failed => return PlayerUpdate::Failed,
            PlayerState::Idle | PlayerState::Paused => return PlayerUpdate::Holding,
            PlayerState::Playing => {}
        }

        let elapsed = self.clock.elapsed();
        let frame_dur = 1.0 / self.fps;
        let pts_of = |f: &VideoFrame, fallback: f64| f.pts_seconds.unwrap_or(fallback);
        let mut advanced = false;

        loop {
            let Some(n) = self.next.as_ref() else { break };
            let cur_pts = self.current.as_ref()
                .map(|f| pts_of(f, self.frames_shown as f64 * frame_dur))
                .unwrap_or(0.0);
            // Never trust a timestamp jump: clamp gaps to one frame duration.
            let mut npts = pts_of(n, cur_pts + frame_dur);
            if npts <= cur_pts || npts > cur_pts + 1.0 {
                npts = cur_pts + frame_dur;
            }
            if npts > elapsed + 1e-3 { break; }

            self.current = self.next.take();
            self.frames_shown += 1;
            advanced = true;

            let t0 = Instant::now();
            self.next = self.decoder.next_frame();
            let dt = t0.elapsed();
            if dt > self.decode_budget && !self.warned_slow {
                self.warned_slow = true;
                log::warn!("video decode exceeded budget ({dt:?} > {:?})", self.decode_budget);
            }
        }

        // End of stream: hold the last frame for one frame slot, then finish.
        if self.next.is_none() {
            let last_pts = self.current.as_ref()
                .map(|f| pts_of(f, self.frames_shown.saturating_sub(1) as f64 * frame_dur));
            let done = self.current.is_none()
                || last_pts.map_or(true, |p| p + frame_dur <= elapsed);
            if done {
                self.state = PlayerState::Finished;
                return PlayerUpdate::Finished;
            }
        }
        if advanced { PlayerUpdate::Advanced } else { PlayerUpdate::Holding }
    }
}