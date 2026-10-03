//! The owl (§14): a video asset, with a static-image fallback. The module is
//! intentionally dumb — load, play, report completion. All transitions belong
//! to animation/. A failed owl NEVER blocks boot (§31): every failure path
//! ends in "finished".

use crate::config::Config;
use crate::error::BootError;
use crate::renderer::{Filter, PixelBuffer, Renderer, Texture};
use crate::video::player::{PlayerUpdate, VideoPlayer};
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwlState { Loading, Playing, Finished, Error }

pub enum Owl {
    Video(VideoOwl),
    Static(StaticOwl),
}

pub struct VideoOwl { player: VideoPlayer, state: OwlState, scale: f32, center_y: f32 }
pub struct StaticOwl { texture: Texture, state: OwlState, until: Instant, scale: f32, center_y: f32 }

impl Owl {
    pub fn open(cfg: &Config, screen_w: u32, screen_h: u32) -> Option<Owl> {
        let _ = (screen_w, screen_h); // geometry is computed per-frame from the buffer
        let scale = cfg.splash.owl_scale;
        let center_y = cfg.splash.owl_center_y;

        // 1) The video asset (the primary path).
        if cfg.splash.owl_video {
            match open_video(cfg) {
                Ok(player) => {
                    return Some(Owl::Video(VideoOwl {
                        player, state: OwlState::Loading, scale, center_y,
                    }));
                }
                Err(e) => log::warn!("owl video unavailable: {e}"),
            }
        }

        // 2) Static image fallback.
        if let Some(path) = cfg.splash.owl_static_fallback.as_ref() {
            match open_static(cfg, path) {
                Ok(texture) => {
                    let hold = Duration::from_millis(cfg.animation.minimum_duration_ms.max(1200) as u64);
                    return Some(Owl::Static(StaticOwl {
                        texture, state: OwlState::Playing, until: Instant::now() + hold,
                        scale, center_y,
                    }));
                }
                Err(e) => log::warn!("owl static fallback unavailable: {e}"),
            }
        }

        log::warn!("no owl asset could be loaded; continuing with wordmark-only splash");
        None
    }

    /// Advance; returns true when the owl presentation is over.
    pub fn update(&mut self, now: Instant) -> bool {
        match self {
            Owl::Video(v) => {
                if v.state == OwlState::Loading { v.state = OwlState::Playing; }
                match v.player.update(now) {
                    PlayerUpdate::Finished => { v.state = OwlState::Finished; true }
                    PlayerUpdate::Failed => {
                        v.state = OwlState::Error;
                        log::warn!("owl video failed mid-playback; continuing boot");
                        true
                    }
                    _ => false,
                }
            }
            Owl::Static(s) => {
                if now >= s.until { s.state = OwlState::Finished; true } else { false }
            }
        }
    }

    pub fn state(&self) -> OwlState {
        match self { Owl::Video(v) => v.state, Owl::Static(s) => s.state }
    }

    pub fn draw(&self, renderer: &Renderer, buf: &mut PixelBuffer, alpha: f32) {
        if alpha <= 0.0 { return; }
        let (sw, sh) = (buf.width(), buf.height());
        match self {
            Owl::Video(v) => {
                if let Some(f) = v.player.frame() {
                    if !f.dimensions_valid() { return; }
                    let (dx, dy, dw, dh) = fit(f.width, f.height, sw, sh, v.scale, v.center_y);
                    renderer.draw_rgba_scaled(
                        buf, &f.pixels, f.width, f.height, dx, dy, dw, dh, alpha, Filter::Bilinear);
                }
            }
            Owl::Static(s) => {
                let (dx, dy, dw, dh) = fit(s.texture.width, s.texture.height, sw, sh, s.scale, s.center_y);
                if dw == 0 || dh == 0 { return; }
                // draw_texture is 1:1; scale the static image via the shared path.
                renderer.draw_rgba_scaled(
                    buf, &s.texture.pixels, s.texture.width, s.texture.height,
                    dx, dy, dw, dh, alpha, Filter::Bilinear);
            }
        }
    }
}

fn open_video(cfg: &Config) -> Result<VideoPlayer, BootError> {
    let path = crate::config::validate_asset_path(&cfg.security, &cfg.splash.owl_video_path)?;
    let mut player = VideoPlayer::open(&path, &cfg.video)?;
    player.play();
    Ok(player)
}

fn open_static(cfg: &Config, path: &Path) -> Result<Texture, BootError> {
    let canon = crate::config::validate_asset_path(&cfg.security, path)?;
    Texture::from_png_file(&canon, cfg.display.max_width.max(4096), cfg.video.max_asset_bytes)
}

/// Aspect-correct fit: `scale` fraction of the shorter screen edge, centered
/// horizontally, anchored at `center_y` vertically.
fn fit(fw: u32, fh: u32, sw: u32, sh: u32, scale: f32, center_y: f32) -> (i32, i32, u32, u32) {
    if fw == 0 || fh == 0 || sw == 0 || sh == 0 { return (0, 0, 0, 0); }
    let target = sw.min(sh) as f32 * scale.clamp(0.05, 1.0);
    let aspect = fw as f32 / fh as f32;
    let (dw, dh) = if aspect >= 1.0 { (target, target / aspect) } else { (target * aspect, target) };
    let dw = dw.min(sw as f32);
    let dh = dh.min(sh as f32);
    let dx = (sw as f32 - dw) * 0.5;
    let dy = (sh as f32 * center_y.clamp(0.0, 1.0)) - dh * 0.5;
    (dx as i32, dy.max(0.0) as i32, dw as u32, dh as u32)
}