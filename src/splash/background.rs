//! Splash background: solid or vertical gradient (§13). Rendered with a fast
//! direct row fill (no per-pixel blend) since it is always fully opaque.

use crate::config::{BackgroundStyle, Config};
use crate::renderer::PixelBuffer;

pub struct Background {
    style: BackgroundStyle,
    solid: u32,
    top: [u8; 3],
    bottom: [u8; 3],
}

impl Background {
    pub fn from_config(cfg: &Config) -> Self {
        let b = &cfg.background;
        let pack = |c: &[u8; 3]| 0xFF00_0000u32 | (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32;
        Background { style: b.style, solid: pack(&b.color), top: b.gradient_top, bottom: b.gradient_bottom }
    }

    pub fn render(&self, buf: &mut PixelBuffer) {
        match self.style {
            BackgroundStyle::Solid => crate::renderer::effects::fill(buf, self.solid),
            BackgroundStyle::Gradient => {
                let h = buf.height().max(1);
                for y in 0..buf.height() {
                    let t = y as f32 / h as f32;
                    let c = lerp3(&self.top, &self.bottom, t);
                    let v = 0xFF00_0000 | (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32;
                    let px = v.to_le_bytes(); // B, G, R, A
                    let row = buf.row_mut(y);
                    for p in row.chunks_exact_mut(4) { p.copy_from_slice(&px); }
                }
            }
        }
    }
}

fn lerp3(a: &[u8; 3], b: &[u8; 3], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        (a[0] as f32 * (1.0 - t) + b[0] as f32 * t) as u8,
        (a[1] as f32 * (1.0 - t) + b[1] as f32 * t) as u8,
        (a[2] as f32 * (1.0 - t) + b[2] as f32 * t) as u8,
    ]
}