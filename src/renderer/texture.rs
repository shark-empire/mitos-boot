//! Owned CPU-side images in the canonical XRGB8888 (0xAARRGGBB) format.

use crate::error::BootError;
use std::path::Path;

#[derive(Clone)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    /// Straight (non-premultiplied) ARGB pixels.
    pub pixels: Vec<u32>,
}

impl Texture {
    pub fn new(width: u32, height: u32) -> Self {
        Texture { width, height, pixels: vec![0; width as usize * height as usize] }
    }

    /// Source-over accumulate into the texture (used when baking the glow).
    #[inline]
    pub fn blend_px(&mut self, x: u32, y: u32, argb: u32, alpha: f32) {
        if x >= self.width || y >= self.height { return; }
        let i = y as usize * self.width as usize + x as usize;
        let (s, d) = (argb, self.pixels[i]);
        let a = alpha.clamp(0.0, 1.0) * ((s >> 24) as f32 / 255.0);
        let da = (d >> 24) as f32 / 255.0;
        let oa = a + da * (1.0 - a);
        if oa <= 0.0 { self.pixels[i] = 0; return; }
        let ch = |sc: u32, dc: u32| -> u32 {
            let v = (sc as f32 * a + dc as f32 * da * (1.0 - a)) / oa;
            v.round().clamp(0.0, 255.0) as u32
        };
        self.pixels[i] = ((oa * 255.0).round() as u32) << 24
            | ch((s >> 16) & 0xFF, (d >> 16) & 0xFF) << 16
            | ch((s >> 8) & 0xFF, (d >> 8) & 0xFF) << 8
            | ch(s & 0xFF, d & 0xFF);
    }

    /// Decode a PNG with hard size limits (dimensions and decoded bytes).
    pub fn from_png_file(path: &Path, max_dim: u32, max_bytes: u64) -> Result<Self, BootError> {
        let meta = std::fs::metadata(path)
            .map_err(|e| BootError::Asset(format!("{}: {e}", path.display())))?;
        if meta.len() > max_bytes {
            return Err(BootError::Asset("png asset exceeds size limit".into()));
        }

        let file = std::fs::File::open(path)
            .map_err(|e| BootError::Asset(format!("{}: {e}", path.display())))?;
        let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().map_err(|e| BootError::Asset(format!("png: {e}")))?;

        let (w, h) = (reader.info().width, reader.info().height);
        if w == 0 || h == 0 || w > max_dim || h > max_dim
            || (w as u64) * (h as u64) * 4 > max_bytes {
            return Err(BootError::Asset(format!("png {w}x{h} exceeds limits")));
        }

        let mut buf = vec![0u8; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).map_err(|e| BootError::Asset(format!("png decode: {e}")))?;

        use png::{BitDepth, ColorType};
        let mut pixels = Vec::with_capacity(w as usize * h as usize);
        match (info.color_type, info.bit_depth) {
            (ColorType::Rgba, BitDepth::Eight) =>
                for c in buf.chunks_exact(4) { pixels.push(u32::from_le_bytes([c[2], c[1], c[0], c[3]])); },
            (ColorType::Rgb, BitDepth::Eight) =>
                for c in buf.chunks_exact(3) { pixels.push(u32::from_le_bytes([c[2], c[1], c[0], 0xFF])); },
            (ColorType::Grayscale, BitDepth::Eight) =>
                for c in buf.chunks_exact(1) { pixels.push(u32::from_le_bytes([c[0], c[0], c[0], 0xFF])); },
            (ColorType::GrayscaleAlpha, BitDepth::Eight) =>
                for c in buf.chunks_exact(2) { pixels.push(u32::from_le_bytes([c[0], c[0], c[0], c[1]])); },
            other => return Err(BootError::Asset(format!("unsupported png output {other:?}"))),
        }
        Ok(Texture { width: w, height: h, pixels })
    }
}