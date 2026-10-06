//! Text rendering.
//!
//! Two independent paths:
//!   1. `rasterize_ttf` — high-quality wordmark glyphs via rusttype.
//!   2. `draw_bitmap*`  — a built-in 5x7 ASCII font that needs no assets and
//!      no allocations. It powers the debug overlay and the error screen, so
//!      the fallback path keeps working even if every other subsystem died.

use super::surface::PixelBuffer;
use super::texture::Texture;

// ---------------------------------------------------------------------------
// Built-in 5x7 font (classic public-domain ASCII set; one byte per column,
// bit 0 = top row). Covers 0x20..=0x7F. Replace the table to restyle.
// ---------------------------------------------------------------------------

const FONT_W: usize = 5;
const FONT_H: usize = 7;

const FONT5X7: [[u8; FONT_W]; 96] = [
    [0x00,0x00,0x00,0x00,0x00], // ' '
    [0x00,0x00,0x5F,0x00,0x00], // '!'
    [0x00,0x07,0x00,0x07,0x00], // '"'
    [0x14,0x7F,0x14,0x7F,0x14], // '#'
    [0x24,0x2A,0x7F,0x2A,0x12], // '$'
    [0x23,0x13,0x08,0x64,0x62], // '%'
    [0x36,0x49,0x55,0x22,0x50], // '&'
    [0x00,0x05,0x03,0x00,0x00], // '\''
    [0x00,0x1C,0x22,0x41,0x00], // '('
    [0x00,0x41,0x22,0x1C,0x00], // ')'
    [0x08,0x2A,0x1C,0x2A,0x08], // '*'
    [0x08,0x08,0x3E,0x08,0x08], // '+'
    [0x00,0x50,0x30,0x00,0x00], // ','
    [0x08,0x08,0x08,0x08,0x08], // '-'
    [0x00,0x60,0x60,0x00,0x00], // '.'
    [0x20,0x10,0x08,0x04,0x02], // '/'
    [0x3E,0x51,0x49,0x45,0x3E], // '0'
    [0x00,0x42,0x7F,0x40,0x00], // '1'
    [0x42,0x61,0x51,0x49,0x46], // '2'
    [0x21,0x41,0x45,0x4B,0x31], // '3'
    [0x18,0x14,0x12,0x7F,0x10], // '4'
    [0x27,0x45,0x45,0x45,0x39], // '5'
    [0x3C,0x4A,0x49,0x49,0x30], // '6'
    [0x01,0x71,0x09,0x05,0x03], // '7'
    [0x36,0x49,0x49,0x49,0x36], // '8'
    [0x06,0x49,0x49,0x29,0x1E], // '9'
    [0x00,0x36,0x36,0x00,0x00], // ':'
    [0x00,0x56,0x36,0x00,0x00], // ';'
    [0x00,0x08,0x14,0x22,0x41], // '<'
    [0x14,0x14,0x14,0x14,0x14], // '='
    [0x41,0x22,0x14,0x08,0x00], // '>'
    [0x02,0x01,0x51,0x09,0x06], // '?'
    [0x32,0x49,0x79,0x41,0x3E], // '@'
    [0x7E,0x11,0x11,0x11,0x7E], // 'A'
    [0x7F,0x49,0x49,0x49,0x36], // 'B'
    [0x3E,0x41,0x41,0x41,0x22], // 'C'
    [0x7F,0x41,0x41,0x22,0x1C], // 'D'
    [0x7F,0x49,0x49,0x49,0x41], // 'E'
    [0x7F,0x09,0x09,0x01,0x01], // 'F'
    [0x3E,0x41,0x41,0x51,0x32], // 'G'
    [0x7F,0x08,0x08,0x08,0x7F], // 'H'
    [0x00,0x41,0x7F,0x41,0x00], // 'I'
    [0x20,0x40,0x41,0x3F,0x01], // 'J'
    [0x7F,0x08,0x14,0x22,0x41], // 'K'
    [0x7F,0x40,0x40,0x40,0x40], // 'L'
    [0x7F,0x02,0x04,0x02,0x7F], // 'M'
    [0x7F,0x04,0x08,0x10,0x7F], // 'N'
    [0x3E,0x41,0x41,0x41,0x3E], // 'O'
    [0x7F,0x09,0x09,0x09,0x06], // 'P'
    [0x3E,0x41,0x51,0x21,0x5E], // 'Q'
    [0x7F,0x09,0x19,0x29,0x46], // 'R'
    [0x46,0x49,0x49,0x49,0x31], // 'S'
    [0x01,0x01,0x7F,0x01,0x01], // 'T'
    [0x3F,0x40,0x40,0x40,0x3F], // 'U'
    [0x1F,0x20,0x40,0x20,0x1F], // 'V'
    [0x7F,0x20,0x18,0x20,0x7F], // 'W'
    [0x63,0x14,0x08,0x14,0x63], // 'X'
    [0x03,0x04,0x78,0x04,0x03], // 'Y'
    [0x61,0x51,0x49,0x45,0x43], // 'Z'
    [0x00,0x00,0x7F,0x41,0x41], // '['
    [0x02,0x04,0x08,0x10,0x20], // '\'
    [0x41,0x41,0x7F,0x00,0x00], // ']'
    [0x04,0x02,0x01,0x02,0x04], // '^'
    [0x40,0x40,0x40,0x40,0x40], // '_'
    [0x00,0x01,0x02,0x04,0x00], // '`'
    [0x20,0x54,0x54,0x54,0x78], // 'a'
    [0x7F,0x48,0x44,0x44,0x38], // 'b'
    [0x38,0x44,0x44,0x44,0x20], // 'c'
    [0x38,0x44,0x44,0x48,0x7F], // 'd'
    [0x38,0x54,0x54,0x54,0x18], // 'e'
    [0x08,0x7E,0x09,0x01,0x02], // 'f'
    [0x08,0x14,0x54,0x54,0x3C], // 'g'
    [0x7F,0x08,0x04,0x04,0x78], // 'h'
    [0x00,0x44,0x7D,0x40,0x00], // 'i'
    [0x20,0x40,0x44,0x3D,0x00], // 'j'
    [0x00,0x7F,0x10,0x28,0x44], // 'k'
    [0x00,0x41,0x7F,0x40,0x00], // 'l'
    [0x7C,0x04,0x18,0x04,0x78], // 'm'
    [0x7C,0x08,0x04,0x04,0x78], // 'n'
    [0x38,0x44,0x44,0x44,0x38], // 'o'
    [0x7C,0x14,0x14,0x14,0x08], // 'p'
    [0x08,0x14,0x14,0x18,0x7C], // 'q'
    [0x7C,0x08,0x04,0x04,0x08], // 'r'
    [0x48,0x54,0x54,0x54,0x20], // 's'
    [0x04,0x3F,0x44,0x40,0x20], // 't'
    [0x3C,0x40,0x40,0x20,0x7C], // 'u'
    [0x1C,0x20,0x40,0x20,0x1C], // 'v'
    [0x3C,0x40,0x30,0x40,0x3C], // 'w'
    [0x44,0x28,0x10,0x28,0x44], // 'x'
    [0x0C,0x50,0x50,0x50,0x3C], // 'y'
    [0x44,0x64,0x54,0x4C,0x44], // 'z'
    [0x00,0x08,0x36,0x41,0x00], // '{'
    [0x00,0x00,0x7F,0x00,0x00], // '|'
    [0x00,0x41,0x36,0x08,0x00], // '}'
    [0x08,0x04,0x08,0x10,0x08], // '~'
    [0x00,0x00,0x00,0x00,0x00], // DEL (unused)
];


pub fn bitmap_text_size(text: &str, px: u32) -> (i32, i32) {
    let s = block_scale(px);
    let n = text.chars().filter(|c| glyph(*c).is_some()).count();
    let w = if n == 0 { 0 } else { n as i32 * (FONT_W as i32 + 1) * s - s };
    (w, FONT_H as i32 * s)
}

fn block_scale(px: u32) -> i32 { (px / FONT_H as u32).clamp(1, 64) as i32 }

fn glyph(c: char) -> Option<&'static [u8; FONT_W]> {
    let i = c as usize;
    if (0x20..0x7F).contains(&i) { Some(&FONT5X7[i - 0x20]) } else { None }
}

/// Draw with the built-in font. Non-ASCII characters are skipped (this path
/// exists precisely so it works with zero assets — see wordmark.rs for the
/// full-Unicode path).
pub fn draw_bitmap(
    buf: &mut PixelBuffer, text: &str, x: i32, y: i32,
    px: u32, color: u32, alpha: f32,
) {
    let s = block_scale(px);
    let mut cx = x;
    for ch in text.chars() {
        let Some(g) = glyph(ch) else { continue };
        for (col, bits) in g.iter().enumerate() {
            for row in 0..FONT_H {
                if bits & (1 << row) != 0 {
                    fill_block(buf, cx + col as i32 * s, y + row as i32 * s, s, color, alpha);
                }
            }
        }
        cx += (FONT_W as i32 + 1) * s;
    }
}

fn fill_block(buf: &mut PixelBuffer, x: i32, y: i32, s: i32, color: u32, alpha: f32) {
    for dy in 0..s {
        for dx in 0..s {
            buf.blend(x + dx, y + dy, color, alpha);
        }
    }
}

pub fn draw_bitmap_centered(
    buf: &mut PixelBuffer, text: &str, cx: f32, cy: f32,
    px: u32, color: u32, alpha: f32,
) {
    let (w, h) = bitmap_text_size(text, px);
    draw_bitmap(buf, text, cx as i32 - w / 2, cy as i32 - h / 2, px, color, alpha);
}

/// ASCII-fallback coverage mask (0.0–1.0 per pixel) for the wordmark bake.
pub fn rasterize_bitmap_coverage(text: &str, px: u32, pad: u32) -> Option<(Vec<f32>, u32, u32)> {
    let s = block_scale(px);
    let filtered: String = text.chars().filter(|c| glyph(*c).is_some()).collect();
    if filtered.is_empty() { return None; }
    let (tw, th) = bitmap_text_size(&filtered, px);
    if tw <= 0 || th <= 0 { return None; }
    let (w, h) = (tw as u32 + pad * 2, th as u32 + pad * 2);
    if w > 4096 || h > 4096 { return None; }

    let mut mask = vec![0.0f32; (w * h) as usize];
    let mut cx = pad as i32;
    for ch in filtered.chars() {
        let g = glyph(ch).unwrap();
        for (col, bits) in g.iter().enumerate() {
            for row in 0..FONT_H {
                if bits & (1 << row) != 0 {
                    for dy in 0..s {
                        for dx in 0..s {
                            let x = cx + col as i32 * s + dx;
                            let y = pad as i32 + row as i32 * s + dy;
                            if x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < h {
                                mask[y as usize * w as usize + x as usize] = 1.0;
                            }
                        }
                    }
                }
            }
        }
        cx += (FONT_W as i32 + 1) * s;
    }
    Some((mask, w, h))
}

/// High-quality coverage mask from a TTF font. This is the path that renders
/// MĨȚǑŠ correctly (the diacritics are non-ASCII). Returns None when the text
/// rasterizes to nothing (e.g. blank string or broken metrics).
pub fn rasterize_ttf_coverage(
    font: &rusttype::Font<'_>, text: &str, px: u32, pad: u32,
) -> Option<(Vec<f32>, u32, u32)> {
    if text.is_empty() { return None; }
    let scale = rusttype::Scale::uniform(px as f32);
    let v = font.v_metrics(scale);
    let glyphs: Vec<_> = font
        .layout(text, scale, rusttype::point(0.0, v.ascent))
        .collect();

    // Global bounding box over all glyphs (accents can overhang the ascent).
    let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
    let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
    for g in &glyphs {
        if let Some(bb) = g.bounding_box() {
            min_x = min_x.min(bb.min.x); min_y = min_y.min(bb.min.y);
            max_x = max_x.max(bb.max.x); max_y = max_y.max(bb.max.y);
        }
    }
    if min_x == f32::MAX || max_x <= min_x || max_y <= min_y { return None; }

    let w = (max_x - min_x).ceil() as u32 + pad * 2;
    let h = (max_y - min_y).ceil() as u32 + pad * 2;
    if w == 0 || h == 0 || w > 4096 || h > 4096 { return None; }

    let mut mask = vec![0.0f32; (w * h) as usize];
    for g in glyphs {
        let Some(bb) = g.bounding_box() else { continue };
        let ox = pad as f32 + (bb.min.x - min_x);
        let oy = pad as f32 + (bb.min.y - min_y);
        g.draw(|x, y, v| {
            let (fx, fy) = (ox + x as f32, oy + y as f32);
            if fx >= 0.0 && fy >= 0.0 && fx < w as f32 && fy < h as f32 {
                let i = fy as usize * w as usize + fx as usize;
                mask[i] = mask[i].max(v); // overlaps: keep max coverage
            }
        });
    }
    Some((mask, w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf4x4() -> ([u8; 64], PixelBuffer<'static>) {
        // SAFETY: the buffer is leaked for the lifetime of the test; the
        // PixelBuffer is only used within the test's single thread.
        let data: &'static mut [u8] = Box::leak(vec![0u8; 64].into_boxed_slice());
        (unsafe { std::mem::transmute::<[u8; 0], [u8; 0]>([]) }, PixelBuffer::from_parts(data, 4, 4, 16))
    }

    #[test]
    fn bitmap_size_is_sane() {
        let (w, h) = bitmap_text_size("MITOS", 28);
        assert!(w > 0 && h > 0 && w > h);
        assert_eq!(bitmap_text_size("", 28), (0, 28));
    }

    #[test]
    fn draw_bitmap_clips_to_buffer() {
        let (_keep, mut b) = buf4x4();
        draw_bitmap(&mut b, "MMMMMMMMMMMM", -100, -100, 28, 0xFFFFFFFF, 1.0);
        // No panic, and the off-canvas majority was simply clipped.
    }

    #[test]
    fn bitmap_coverage_has_pixels() {
        let (mask, w, h) = rasterize_bitmap_coverage("MITOS", 28, 4).unwrap();
        assert_eq!(mask.len(), (w * h) as usize);
        assert!(mask.iter().any(|&v| v > 0.0));
    }
}