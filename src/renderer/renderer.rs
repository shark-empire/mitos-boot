//! Main rendering interface (§8).
//!
//! The renderer draws things; the splash system decides what those things
//! represent. It stays independent of boot state.

use super::compositor;
use super::surface::PixelBuffer;
use super::texture::Texture;
use crate::config::{self, Config};
use crate::splash::background::Background;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter { Nearest, Bilinear }

pub struct Renderer {
    font: Option<rusttype::Font<'static>>,
    filter: Filter,
    frames: AtomicU64,
}

impl Renderer {
    pub fn new(cfg: &Config) -> Renderer {
        let font = Self::load_font(cfg);
        if font.is_none() {
            log::warn!("no wordmark font available; bitmap fallback text will be used");
        }
        Renderer { font, filter: Filter::Bilinear, frames: AtomicU64::new(0) }
    }

    /// Load the configured display font, honoring the asset allowlist and a
    /// hard size cap.
    fn load_font(cfg: &Config) -> Option<rusttype::Font<'static>> {
        let mut candidates: Vec<std::path::PathBuf> = vec![cfg.splash.wordmark_font.clone()];
        candidates.extend(cfg.splash.font_fallbacks.iter().cloned());

        for path in candidates {
            let Ok(canon) = config::validate_asset_path(&cfg.security, &path) else { continue };
            let Ok(meta) = std::fs::metadata(&canon) else { continue };
            if meta.len() > (64 << 20) { continue; } // 64 MiB font cap
            let Ok(data) = std::fs::read(&canon) else { continue };
            if let Some(f) = rusttype::Font::try_from_vec(data) { return Some(f); }
            log::debug!("font {} failed to parse", canon.display());
        }
        None
    }

    pub fn font(&self) -> Option<&rusttype::Font<'static>> { self.font.as_ref() }

    pub fn begin_frame(&self) { self.frames.fetch_add(1, Ordering::Relaxed); }
    pub fn end_frame(&self) {}
    #[allow(dead_code)] // useful diagnostic for future phases
    pub fn frame_count(&self) -> u64 { self.frames.load(Ordering::Relaxed) }

    pub fn draw_background(&self, buf: &mut PixelBuffer, bg: &Background) {
        bg.render(buf);
    }

    pub fn draw_texture(&self, buf: &mut PixelBuffer, tex: &Texture, dx: i32, dy: i32, alpha: f32) {
        compositor::blit(buf, tex, dx, dy, alpha);
    }

    pub fn draw_rgba_scaled(
        &self, buf: &mut PixelBuffer, src: &[u32], sw: u32, sh: u32,
        dx: i32, dy: i32, dw: u32, dh: u32, alpha: f32, filter: Filter,
    ) {
        match filter {
            Filter::Nearest =>
                compositor::blit_scaled_nearest(buf, src, sw, sh, dx, dy, dw, dh, alpha),
            Filter::Bilinear =>
                compositor::blit_scaled_bilinear(buf, src, sw, sh, dx, dy, dw, dh, alpha),
        }
    }

    #[allow(dead_code)]
    pub fn filter(&self) -> Filter { self.filter }
}