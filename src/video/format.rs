//! Container sniffing, the raw MITOSV asset format, and pixel conversion.
//!
//! MITOSV — a deliberately dumb, dependency-free animation container for
//! builds without FFmpeg and for tool-generated assets:
//!
//!   offset  size  field
//!   0       8     magic  b"MITOSV1\0"
//!   8       4     width      (u32 LE, 1..=8192)
//!   12      4     height     (u32 LE, 1..=8192)
//!   16      4     fps        (u32 LE, 1..=240)
//!   20      4     frame_count(u32 LE, 1..=100000)
//!   24      4     format     (u32 LE; 0 = RGBA8, bytes R,G,B,A)
//!   28      ...   frames, width*height*4 bytes each, in display order
//!
//! Every field is validated before a single frame is decoded; total asset
//! size is capped by config (video.max_asset_bytes).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub const MITOSV_MAGIC: [u8; 8] = *b"MITOSV1\0";
pub const MITOSV_HEADER_LEN: usize = 28;
pub const MAX_DIM: u32 = 8192;
pub const MAX_FPS: u32 = 240;
pub const MAX_FRAMES: u32 = 100_000;
pub const MAX_FRAME_BYTES: usize = 64 << 20; // per-frame sanity cap
pub const FORMAT_RGBA_LE: u32 = 0;

pub fn read_exact_n(f: &mut File, n: usize) -> std::io::Result<Vec<u8>> {
    let mut v = vec![0u8; n];
    f.read_exact(&mut v)?;
    Ok(v)
}

pub fn le_u32(b: &[u8]) -> u32 {
    u32::from_le_bytes(b.try_into().expect("4-byte slice"))
}

/// True when the file starts with the MITOSV magic. Rewinds on exit.
pub fn sniff_mitosv(f: &mut File) -> bool {
    let ok = read_exact_n(f, 8).map(|m| m == MITOSV_MAGIC).unwrap_or(false);
    let _ = f.seek(SeekFrom::Start(0));
    ok
}

pub fn has_ffmpeg_extension(p: &Path) -> bool {
    matches!(
        p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("webm" | "mkv" | "mp4" | "mov" | "avi" | "ogv" | "ivf")
    )
}

/// RGBA8 bytes (R,G,B,A per pixel) → canonical `0xAARRGGBB` pixels.
/// Length is validated before any allocation assumption is trusted.
pub fn pixels_from_rgba_le(bytes: &[u8], expected_pixels: usize) -> Option<Vec<u32>> {
    if bytes.len() != expected_pixels * 4 { return None; }
    Some(bytes.chunks_exact(4).map(|c| u32::from_le_bytes([c[2], c[1], c[0], c[3]])).collect())
}