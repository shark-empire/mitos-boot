//! The MĨȚǑŠ wordmark (§15). Rasterized once into a texture (glow baked in),
//! then blitted per frame — zero per-frame text cost.
//!
//! Unicode (MĨȚǑŠ has diacritics) requires the TTF path; without a font the
//! ASCII bitmap fallback renders a plain "MITOS" rather than failing.

use crate::config::Config;
use crate::error::BootError;
use crate::renderer::text;
use crate::renderer::{effects, PixelBuffer, Renderer, Texture};

pub struct Wordmark {
    texture: Texture,
    dx: i32,
    dy: i32,
}

impl Wordmark {
    pub fn build(cfg: &Config, renderer: &Renderer, screen_w: u32, screen_h: u32)
        -> Result<Wordmark, BootError>
    {
        // Reference size is 1080p; scale with panel height, clamped.
        let px = (cfg.splash.wordmark_size_px as f32 * (screen_h as f32 / 1080.0))
            .clamp(24.0, 512.0) as u32;
        let pad = (px / 6).max(8) as u32;

        let coverage = renderer
            .font()
            .and_then(|f| text::rasterize_ttf_coverage(f, &cfg.splash.wordmark_text, px, pad))
            .or_else(|| {
                log::warn!("wordmark font unavailable; using ASCII fallback glyphs");
                text::rasterize_bitmap_coverage(&cfg.splash.wordmark_text, px, pad)
            });

        let Some((mask, mw, mh)) = coverage else {
            return Err(BootError::Renderer("could not rasterize wordmark text".into()));
        };

        let radius = (px as usize / 24).clamp(2, 12);
        let texture = bake(&mask, mw, mh, cfg, radius);

        let dx = (screen_w as i64 - mw as i64) / 2;
        let dy = (screen_h as f64 * cfg.splash.wordmark_center_y.clamp(0.0, 1.0) as f64
                  - mh as f64 / 2.0).round() as i64;

        Ok(Wordmark { texture, dx: dx.max(0) as i32, dy: dy.max(0) as i32 })
    }

    pub fn draw(&self, renderer: &Renderer, buf: &mut PixelBuffer, alpha: f32) {
        renderer.draw_texture(buf, &self.texture, self.dx, self.dy, alpha);
    }
}

fn argb(c: &[u8; 3], a: f32) -> u32 {
    ((a.clamp(0.0, 1.0) * 255.0).round() as u32) << 24
        | (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32
}

/// Bake text (+ optional glow) into a texture exactly once.
fn bake(mask: &[f32], w: u32, h: u32, cfg: &Config, radius: usize) -> Texture {
    const TEXT: [u8; 3] = [0xF2, 0xF5, 0xFA]; // near-white
    let glow_strength = if cfg.splash.wordmark_glow { 0.85f32 } else { 0.0 };
    let glow_color = cfg.splash.glow_color;

    let mut glow_mask = mask.to_vec();
    if glow_strength > 0.0 {
        effects::box_blur(&mut glow_mask, w as usize, h as usize, radius);
        effects::box_blur(&mut glow_mask, w as usize, h as usize, radius.max(2));
    }

    let mut tex = Texture::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let i = y as usize * w as usize + x as usize;
            let m = mask[i].clamp(0.0, 1.0);
            let g = (glow_mask[i] * glow_strength).clamp(0.0, 1.0);
            // Base layer: soft glow halo.
            tex.pixels[i] = argb(&glow_color, g * 0.85);
            // Text over the glow (proper source-over).
            if m > 0.0 { tex.blend_px(x, y, argb(&TEXT, m), 1.0); }
        }
    }
    tex
}