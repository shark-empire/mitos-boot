//! A single decoded video frame in the renderer's canonical pixel format.

/// Pixels are straight-alpha `0xAARRGGBB` (little-endian memory order B,G,R,A)
/// — identical to `renderer::PixelBuffer` and DRM XRGB8888.
#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
    /// Presentation time in seconds from video start, when known.
    pub pts_seconds: Option<f64>,
}

impl VideoFrame {
    pub fn dimensions_valid(&self) -> bool {
        self.width > 0
            && self.height > 0
            && self.width <= 8192
            && self.height <= 8192
            && self.pixels.len() == self.width as usize * self.height as usize
    }
}