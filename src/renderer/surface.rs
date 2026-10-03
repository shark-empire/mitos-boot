//! Pixel surface primitives.
//!
//! Canonical in-memory format: XRGB8888 stored as native-endian u32
//! (0xAARRGGBB), i.e. bytes B,G,R,A on little-endian hosts — byte-identical
//! to DRM_FORMAT_XRGB8888 and the common fbdev layout.

/// Integer rectangle with signed origin (clipping-friendly).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect { pub x: i32, pub y: i32, pub w: u32, pub h: u32 }

impl Rect {
    /// Clip against the buffer; returns inclusive-exclusive pixel bounds.
    pub fn clip(&self, width: u32, height: u32) -> (i32, i32, i32, i32) {
        let x0 = self.x.max(0);
        let y0 = self.y.max(0);
        let x1 = ((self.x as i64 + self.w as i64).min(width as i64)).min(i32::MAX as i64) as i32;
        let y1 = ((self.y as i64 + self.h as i64).min(height as i64)).min(i32::MAX as i64) as i32;
        (x0, y0, x1.max(x0), y1.max(y0))
    }
}

pub struct PixelBuffer<'a> {
    data: &'a mut [u8],
    width: u32,
    height: u32,
    stride: usize,
}

impl<'a> PixelBuffer<'a> {
    pub fn from_parts(data: &'a mut [u8], width: u32, height: u32, stride: usize) -> Self {
        debug_assert!(stride >= width as usize * 4);
        debug_assert!(data.len() >= stride.saturating_mul(height as usize));
        PixelBuffer { data, width, height, stride }
    }

    pub fn width(&self) -> u32 { self.width }
    pub fn height(&self) -> u32 { self.height }
    pub fn stride(&self) -> usize { self.stride }

    #[inline]
    pub fn row_mut(&mut self, y: u32) -> &mut [u8] {
        let off = y as usize * self.stride;
        &mut self.data[off .. off + self.stride]
    }

    #[inline]
    pub fn px(&self, x: u32, y: u32) -> u32 {
        if x >= self.width || y >= self.height { return 0; }
        let off = y as usize * self.stride + x as usize * 4;
        u32::from_le_bytes(self.data[off..off + 4].try_into().unwrap())
    }

    #[inline]
    pub fn set_px(&mut self, x: u32, y: u32, v: u32) {
        if x >= self.width || y >= self.height { return; }
        let off = y as usize * self.stride + x as usize * 4;
        self.data[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// Source-over blend at signed coordinates (negatives clipped away).
    #[inline]
    pub fn blend(&mut self, x: i32, y: i32, argb: u32, alpha: f32) {
        if x < 0 || y < 0 { return; }
        self.blend_u(x as u32, y as u32, argb, alpha);
    }

    #[inline]
    pub fn blend_u(&mut self, x: u32, y: u32, argb: u32, alpha: f32) {
        if x >= self.width || y >= self.height { return; }
        let a = alpha.clamp(0.0, 1.0) * ((argb >> 24) as f32 / 255.0);
        if a <= 0.0 { return; }
        let off = y as usize * self.stride + x as usize * 4;
        let d = &mut self.data[off..off + 4];
        if a >= 1.0 {
            d[0] = (argb & 0xFF) as u8;
            d[1] = ((argb >> 8) & 0xFF) as u8;
            d[2] = ((argb >> 16) & 0xFF) as u8;
            d[3] = 0xFF;
            return;
        }
        let inv = 1.0 - a;
        for (i, sv) in [(0usize, argb & 0xFF), (1, (argb >> 8) & 0xFF), (2, (argb >> 16) & 0xFF)] {
            d[i] = (sv as f32 * a + d[i] as f32 * inv + 0.5) as u8;
        }
        d[3] = 0xFF;
    }
}