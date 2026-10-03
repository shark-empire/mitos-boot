//! Full-frame effects and the reusable mask blur used to bake the wordmark
//! glow. Everything here is one-time-cost friendly (bake, don't per-frame).

use super::surface::PixelBuffer;

/// Opaque whole-buffer fill (fast path used for the black boot frame and the
/// error screen).
pub fn fill(buf: &mut PixelBuffer, argb: u32) {
    let px = argb.to_le_bytes(); // B, G, R, A
    for y in 0..buf.height() {
        let row = buf.row_mut(y);
        for p in row.chunks_exact_mut(4) {
            p.copy_from_slice(&px);
        }
    }
}

/// Two-pass (horizontal + vertical) box blur with edge clamping. Used on
/// coverage masks at bake time; radius is expected to stay small (≤ ~16).
pub fn box_blur(data: &mut [f32], w: usize, h: usize, radius: usize) {
    if w == 0 || h == 0 || radius == 0 || data.len() < w * h { return; }
    let mut tmp = vec![0.0f32; w * h];

    for y in 0..h {
        for x in 0..w {
            let mut sum = 0.0f32;
            let mut n = 0.0f32;
            for k in -(radius as isize)..=(radius as isize) {
                let xx = (x as isize + k).clamp(0, w as isize - 1) as usize;
                sum += data[y * w + xx];
                n += 1.0;
            }
            tmp[y * w + x] = sum / n;
        }
    }
    for x in 0..w {
        for y in 0..h {
            let mut sum = 0.0f32;
            let mut n = 0.0f32;
            for k in -(radius as isize)..=(radius as isize) {
                let yy = (y as isize + k).clamp(0, h as isize - 1) as usize;
                sum += tmp[yy * w + x];
                n += 1.0;
            }
            data[y * w + x] = sum / n;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blur_spreads_a_point() {
        let mut m = vec![0.0f32; 9 * 9];
        m[4 * 9 + 4] = 1.0;
        box_blur(&mut m, 9, 9, 1);
        assert!(m[3 * 9 + 4] > 0.0 && m[4 * 9 + 3] > 0.0);
        assert!(m[4 * 9 + 4] < 1.0); // energy spread out
    }
}