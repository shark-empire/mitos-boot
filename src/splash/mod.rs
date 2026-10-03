//! The splash system controls MITOS's visual identity (§13): owl, wordmark,
//! background. Construction degrades gracefully at every step — a broken
//! video falls back to a static image, then to a wordmark-only splash.

pub mod background;
pub mod owl;
pub mod wordmark;

pub use background::Background;

use crate::config::Config;
use crate::renderer::{PixelBuffer, Renderer};
use std::time::Instant;

pub struct SplashScene {
    pub owl: Option<owl::Owl>,
    pub wordmark: Option<wordmark::Wordmark>,
    pub background: Background,
}

impl SplashScene {
    pub fn build(cfg: &Config, renderer: &Renderer, screen_w: u32, screen_h: u32) -> SplashScene {
        let owl = if cfg.splash.owl_video { owl::Owl::open(cfg, screen_w, screen_h) } else { None };
        let wordmark = if cfg.splash.wordmark {
            match wordmark::Wordmark::build(cfg, renderer, screen_w, screen_h) {
                Ok(w) => Some(w),
                Err(e) => { log::warn!("wordmark unavailable: {e}"); None }
            }
        } else { None };
        SplashScene { owl, wordmark, background: Background::from_config(cfg) }
    }

    /// Advance the scene; true when the owl presentation is over.
    pub fn update(&mut self, now: Instant) -> bool {
        match self.owl.as_mut() {
            Some(o) => o.update(now),
            None => true,
        }
    }

    pub fn draw_owl(&self, renderer: &Renderer, buf: &mut PixelBuffer, alpha: f32) {
        if let Some(o) = self.owl.as_ref() { o.draw(renderer, buf, alpha); }
    }

    pub fn draw_wordmark(&self, renderer: &Renderer, buf: &mut PixelBuffer, alpha: f32) {
        if let Some(w) = self.wordmark.as_ref() { w.draw(renderer, buf, alpha); }
    }
}