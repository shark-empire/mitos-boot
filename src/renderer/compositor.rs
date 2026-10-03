//! Pixel compositing: fills, blits, scaled blits (nearest + bilinear).

use super::surface::{PixelBuffer, Rect};
use super::texture::Texture;

pub fn fill_rect(buf: &mut PixelBuffer, r: &Rect, color: u32, alpha: f32) {
    let (x0, y0, x1, y1) = r.clip(buf.width(), buf.height());
    for y in y0..y1 {
        for x in x0..x1 {
            buf.blend_u(x as u32, y as u32, color, alpha);
        }
    }
}

/// 1:1 blit with per-pixel alpha.
pub fn blit(buf: &mut PixelBuffer, tex: &Texture, dx: i32, dy: i32, alpha: f32) {
    blit_scaled_nearest(buf, &tex.pixels, tex.width, tex.height,
                        dx, dy, tex.width, tex.height, alpha);
}

pub fn blit_scaled_nearest(
    buf: &mut PixelBuffer, src: &[u32], sw: u32, sh: u32,
    dx: i32, dy: i32, dw: u32, dh: u32, alpha: f32,
) {
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 || src.len() < (sw * sh) as usize { return; }
    let (x0, y0, x1, y1) = Rect { x: dx, y: dy, w: dw, h: dh }.clip(buf.width(), buf.height());
    for y in y0..y1 {
        let sy = (((y - dy) as u64 * sh as u64 / dw.max(1) as u64).min(sh as u64 - 1)) as u32;
        for x in x0..x1 {
            let sx = (((x - dx) as u64 * sw as u64 / dw.max(1) as u64).min(sw as u64 - 1)) as u32;
            buf.blend_u(x as u32, y as u32, src[sy as usize * sw as usize + sx as usize], alpha);
        }
    }
}

pub fn blit_scaled_bilinear(
    buf: &mut PixelBuffer, src: &[u32], sw: u32, sh: u32,
    dx: i32, dy: i32, dw: u32, dh: u32, alpha: f32,
) {
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 || src.len() < (sw * sh) as usize { return; }
    if dw == sw && dh == sh {
        return blit_scaled_nearest(buf, src, sw, sh, dx, dy, dw, dh, alpha);
    }
    let (x0, y0, x1, y1) = Rect { x: dx, y: dy, w: dw, h: dh }.clip(buf.width(), buf.height());
    let x_ratio = sw as f32 / dw as f32;
    let y_ratio = sh as f32 / dh as f32;

    for y in y0..y1 {
        let fy = (y - dy) as f32 * y_ratio - 0.5;
        let sy0 = (fy.floor().max(0.0) as u32).min(sh - 1);
        let sy1 = (sy0 + 1).min(sh - 1);
        let ty = (fy - fy.floor()).clamp(0.0, 1.0);
        for x in x0..x1 {
            let fx = (x - dx) as f32 * x_ratio - 0.5;
            let sx0 = (fx.floor().max(0.0) as u32).min(sw - 1);
            let sx1 = (sx0 + 1).min(sw - 1);
            let tx = (fx - fx.floor()).clamp(0.0, 1.0);

            let at = |sx: u32, sy: u32| src[sy as usize * sw as usize + sx as usize];
            let lerp = |a: u32, b: u32, t: f32| {
                let ch = |sh: u32| ((a >> sh & 0xFF) as f32 * (1.0 - t) + (b >> sh & 0xFF) as f32 * t) as u32;
                (ch(24) << 24) | (ch(16) << 16) | (ch(8) << 8) | ch(0)
            };
            let c = lerp(lerp(at(sx0, sy0), at(sx1, sy0), tx),
                         lerp(at(sx0, sy1), at(sx1, sy1), tx), ty);
            buf.blend_u(x as u32, y as u32, c, alpha);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_rect_clips_and_blends() {
        let mut data = [0u8; 4 * 4 * 4];
        let mut buf = PixelBuffer::from_parts(&mut data, 4, 4, 16);

        // Rectangle partly off-screen: only the on-screen part is drawn.
        fill_rect(&mut buf, &Rect { x: 2, y: 2, w: 10, h: 10 }, 0xFF00FF00, 1.0);
        assert_eq!(buf.px(3, 3), 0xFF00FF00);
        assert_eq!(buf.px(0, 0), 0);

        // 50% white over untouched (black) background.
        fill_rect(&mut buf, &Rect { x: 0, y: 0, w: 1, h: 1 }, 0x80FFFFFF, 1.0);
        assert_eq!(buf.px(0, 0), 0xFF808080);
    }
}