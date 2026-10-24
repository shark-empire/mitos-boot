//! Decoder backends (§11, §12).
//!
//! `open()` sniffs the asset: MITOSV magic → MITOSV decoder (raw or paletted;
//! always available, no FFmpeg);
//! anything else → FFmpeg decoder when the `video-ffmpeg` feature is enabled.
//! A decoder failure is always an error the *caller* can fall back from —
//! a broken video must never block boot.

use super::format;
use super::frame::VideoFrame;
use crate::config::VideoSection;
use crate::error::BootError;
use std::path::Path;
use std::io::{Read, Seek, SeekFrom};

#[derive(Debug, Clone)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub estimated_fps: f64,
    pub frame_count: Option<u64>,
    pub duration_seconds: Option<f64>,
}

pub trait VideoDecoder {
    fn info(&self) -> &VideoInfo;
    /// Decode and return the next frame in display order; None at end.
    fn next_frame(&mut self) -> Option<VideoFrame>;
    fn finished(&self) -> bool;
    /// Restart from the first frame (loop playback). Returns false when this
    /// backend cannot rewind; callers then treat the video as finished.
    fn rewind(&mut self) -> bool { false }
}

pub fn open(path: &Path, cfg: &VideoSection) -> Result<Box<dyn VideoDecoder>, BootError> {
    let meta = std::fs::metadata(path)
        .map_err(|e| BootError::Video(format!("{}: {e}", path.display())))?;
    if !meta.is_file() {
        return Err(BootError::Video("asset is not a regular file".into()));
    }
    if meta.len() > cfg.max_asset_bytes {
        return Err(BootError::Video("asset exceeds configured size limit".into()));
    }

    let mut file = std::fs::File::open(path)
        .map_err(|e| BootError::Video(format!("{}: {e}", path.display())))?;

    if format::sniff_mitosv(&mut file) {
        return Ok(Box::new(MitosvDecoder::open(file)?));
    }
    drop(file);

    #[cfg(feature = "video-ffmpeg")]
    {
        return Ok(Box::new(ff::FfmpegDecoder::open(path)?));
    }
    #[cfg(not(feature = "video-ffmpeg"))]
    {
        if !format::has_ffmpeg_extension(path) {
            return Err(BootError::Video(format!(
                "{}: unknown asset format (expected MITOSV or a video container)",
                path.display()
            )));
        }
        return Err(BootError::Video(format!(
            "{}: this build has no FFmpeg decoder; rebuild with --features video-ffmpeg",
            path.display()
        )));
    }
}

// ---------------------------------------------------------------------------
// MITOSV decoder — dependency-light, fully validated, streaming (one frame in
// memory at a time). Format 0 is raw RGBA8; format 1 is the compact paletted
// zlib-delta format the boot owl ships in (see format.rs).
// ---------------------------------------------------------------------------

/// Extra state for `FORMAT_PAL8_ZDELTA` assets.
struct PalState {
    /// 0xFFRRGGBB per index. Entries past the file's palette stay opaque
    /// black, so a corrupt index can never address outside the table.
    palette: [u32; 256],
    /// Palette indices of the frame on screen: the base for the next delta.
    indices: Vec<u8>,
    /// Inflate target, reused for every frame.
    delta: Vec<u8>,
    /// Largest compressed chunk we are willing to read.
    max_chunk: usize,
}

struct MitosvDecoder {
    file: std::fs::File,
    data_start: u64,
    frame_bytes: usize,
    frame_count: u32,
    frames_read: u32,
    fps: f64,
    info: VideoInfo,
    scratch: Vec<u8>,
    pal: Option<PalState>,
    finished: bool,
}

/// Read the format-1 palette header; returns the table and bytes consumed.
fn read_palette(file: &mut std::fs::File) -> Result<([u32; 256], u64), BootError> {
    let bad = |m: String| BootError::Video(format!("mitosv: {m}"));
    let head = format::read_exact_n(file, 2).map_err(|e| bad(format!("palette header: {e}")))?;
    let n = u16::from_le_bytes([head[0], head[1]]) as usize;
    if n == 0 || n > 256 {
        return Err(bad("palette size out of range".into()));
    }
    let rgb = format::read_exact_n(file, n * 3).map_err(|e| bad(format!("palette: {e}")))?;
    let mut palette = [0xFF00_0000u32; 256];
    for (slot, c) in palette.iter_mut().zip(rgb.chunks_exact(3)) {
        *slot = 0xFF00_0000 | ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | (c[2] as u32);
    }
    Ok((palette, 2 + 3 * (n as u64)))
}

/// Decode one format-1 chunk into canonical 0xAARRGGBB pixels. `None` means
/// the asset is truncated or corrupt. The delta base only advances after the
/// whole chunk inflated successfully, so a bad chunk never leaves half a frame.
fn read_pal_frame(
    file: &mut std::fs::File,
    scratch: &mut Vec<u8>,
    p: &mut PalState,
) -> Option<Vec<u32>> {
    let mut len4 = [0u8; 4];
    file.read_exact(&mut len4).ok()?;
    let zlen = u32::from_le_bytes(len4) as usize;
    if zlen == 0 || zlen > p.max_chunk {
        return None;
    }
    scratch.clear();
    scratch.resize(zlen, 0);
    file.read_exact(&mut scratch[..]).ok()?;

    // Output is bounded by the plane size: a hostile stream cannot inflate
    // past `delta`, and a short one is an error.
    let mut z = flate2::read::ZlibDecoder::new(&scratch[..]);
    z.read_exact(&mut p.delta[..]).ok()?;

    let mut pixels = Vec::with_capacity(p.indices.len());
    for (ix, &d) in p.indices.iter_mut().zip(p.delta.iter()) {
        *ix = ix.wrapping_add(d);
        pixels.push(p.palette[*ix as usize]);
    }
    Some(pixels)
}

impl MitosvDecoder {
    fn open(mut file: std::fs::File) -> Result<Self, BootError> {
        let hdr = format::read_exact_n(&mut file, format::MITOSV_HEADER_LEN)
            .map_err(|e| BootError::Video(format!("mitosv header: {e}")))?;
        if hdr.len() != format::MITOSV_HEADER_LEN || hdr[..8] != format::MITOSV_MAGIC {
            return Err(BootError::Video("mitosv: bad header".into()));
        }
        let w = format::le_u32(&hdr[8..12]);
        let h = format::le_u32(&hdr[12..16]);
        let fps = format::le_u32(&hdr[16..20]);
        let frames = format::le_u32(&hdr[20..24]);
        let fmt = format::le_u32(&hdr[24..28]);

        if w == 0 || h == 0 || w > format::MAX_DIM || h > format::MAX_DIM {
            return Err(BootError::Video("mitosv: dimensions out of range".into()));
        }
        if fps == 0 || fps > format::MAX_FPS {
            return Err(BootError::Video("mitosv: fps out of range".into()));
        }
        if frames == 0 || frames > format::MAX_FRAMES {
            return Err(BootError::Video("mitosv: frame count out of range".into()));
        }
        if fmt != format::FORMAT_RGBA_LE && fmt != format::FORMAT_PAL8_ZDELTA {
            return Err(BootError::Video("mitosv: unsupported pixel format".into()));
        }

        // Bounds the u32 pixel buffer of every format (and the raw frame size).
        let frame_bytes = (w as usize)
            .checked_mul(h as usize)
            .and_then(|p| p.checked_mul(4))
            .filter(|&b| b <= format::MAX_FRAME_BYTES)
            .ok_or_else(|| BootError::Video("mitosv: frame size out of range".into()))?;

        let mut data_start = format::MITOSV_HEADER_LEN as u64;
        let pal = if fmt == format::FORMAT_PAL8_ZDELTA {
            let (palette, used) = read_palette(&mut file)?;
            data_start += used;
            let px = w as usize * h as usize;
            Some(PalState {
                palette,
                indices: vec![0u8; px],
                delta: vec![0u8; px],
                max_chunk: px + px / 8 + 1024,
            })
        } else {
            let expected = data_start + frame_bytes as u64 * frames as u64;
            let actual = file.metadata()
                .map_err(|e| BootError::Video(format!("mitosv: {e}")))?.len();
            if actual < expected {
                return Err(BootError::Video("mitosv: file truncated".into()));
            }
            None
        };

        let info = VideoInfo {
            width: w,
            height: h,
            estimated_fps: fps as f64,
            frame_count: Some(frames as u64),
            duration_seconds: Some(frames as f64 / fps as f64),
        };
        Ok(MitosvDecoder {
            file, data_start, frame_bytes, frame_count: frames, frames_read: 0,
            fps: fps as f64, info, scratch: Vec::new(), pal, finished: false,
        })
    }
}

impl VideoDecoder for MitosvDecoder {
    fn info(&self) -> &VideoInfo { &self.info }
    fn finished(&self) -> bool { self.finished }

    fn next_frame(&mut self) -> Option<VideoFrame> {
        if self.finished || self.frames_read >= self.frame_count {
            self.finished = true;
            return None;
        }
        let pixels = match self.pal.as_mut() {
            Some(p) => read_pal_frame(&mut self.file, &mut self.scratch, p),
            None => {
                self.scratch.clear();
                self.scratch.resize(self.frame_bytes, 0);
                if self.file.read_exact(&mut self.scratch).is_err() {
                    None
                } else {
                    format::pixels_from_rgba_le(
                        &self.scratch, self.info.width as usize * self.info.height as usize)
                }
            }
        };
        let Some(pixels) = pixels else {
            self.finished = true; // truncated or corrupt mid-asset: stop cleanly
            return None;
        };
        let pts = Some(self.frames_read as f64 / self.fps);
        self.frames_read += 1;
        if self.frames_read >= self.frame_count { self.finished = true; }
        Some(VideoFrame { width: self.info.width, height: self.info.height, pixels, pts_seconds: pts })
    }

    fn rewind(&mut self) -> bool {
        if self.file.seek(SeekFrom::Start(self.data_start)).is_err() {
            return false;
        }
        self.frames_read = 0;
        self.finished = false;
        if let Some(p) = self.pal.as_mut() {
            p.indices.fill(0); // frame 0 is a keyframe against all-zero indices
        }
        true
    }
}

// ---------------------------------------------------------------------------
// FFmpeg decoder (feature-gated). Decodes WebM/VP9/AV1/… and converts to
// canonical pixels via swscale into BGRA (byte order B,G,R,A == 0xAARRGGBB LE,
// so no per-pixel swizzle is needed).
//
// Compatibility note: written against ffmpeg-next 7.x, whose `packets()`
// iterator yields Result<(Stream, Packet), Error>. On older ffmpeg-next that
// yields plain tuples, drop the Ok(...)/Err(_) arms accordingly.
// ---------------------------------------------------------------------------

#[cfg(feature = "video-ffmpeg")]
mod ff {
    use super::{VideoDecoder, VideoInfo};
    use crate::error::BootError;
    use crate::video::frame::VideoFrame;
    use ffmpeg_next as ffmpeg;
    use std::path::Path;

    static INIT: std::sync::Once = std::sync::Once::new();

    pub struct FfmpegDecoder {
        ictx: ffmpeg::format::context::Input,
        decoder: ffmpeg::codec::decoder::Video,
        scaler: Option<ffmpeg::software::scaling::Context>,
        stream_index: usize,
        time_base_s: f64,
        info: VideoInfo,
        sent_eof: bool,
        finished: bool,
    }

    impl FfmpegDecoder {
        pub fn open(path: &Path) -> Result<Self, BootError> {
            INIT.call_once(|| {
                if let Err(e) = ffmpeg::init() {
                    log::warn!("ffmpeg init failed: {e}");
                }
            });

            let ictx = ffmpeg::format::input(path)
                .map_err(|e| BootError::Video(format!("open {}: {e}", path.display())))?;
            let stream = ictx.streams().best(ffmpeg::media::Type::Video)
                .ok_or_else(|| BootError::Video("no video stream in asset".into()))?;

            let stream_index = stream.index();
            let tb = stream.time_base();
            let time_base_s = if tb.1 != 0 { tb.0 as f64 / tb.1 as f64 } else { 0.0 };
            let rate = stream.rate();
            let fps = if rate.1 != 0 && rate.0 > 0 {
                (rate.0 as f64 / rate.1 as f64).clamp(1.0, 240.0)
            } else { 30.0 };
            let frames = stream.frames();
            let dur_tb = stream.duration();
            let duration = if dur_tb > 0 && time_base_s > 0.0 {
                Some(dur_tb as f64 * time_base_s)
            } else { None };

            let ctx = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
                .map_err(|e| BootError::Video(format!("codec: {e}")))?;
            let decoder = ctx.decoder().video()
                .map_err(|e| BootError::Video(format!("video decoder: {e}")))?;

            let (w, h) = (decoder.width(), decoder.height());
            if w == 0 || h == 0 || w > 8192 || h > 8192 {
                return Err(BootError::Video("implausible video dimensions".into()));
            }

            let info = VideoInfo {
                width: w, height: h, estimated_fps: fps,
                frame_count: (frames > 0).then_some(frames as u64),
                duration_seconds: duration,
            };
            Ok(FfmpegDecoder { ictx, decoder, scaler: None, stream_index, time_base_s,
                               info, sent_eof: false, finished: false })
        }
    }

    impl VideoDecoder for FfmpegDecoder {
        fn info(&self) -> &VideoInfo { &self.info }
        fn finished(&self) -> bool { self.finished }

        fn next_frame(&mut self) -> Option<VideoFrame> {
            if self.finished { return None; }
            // Split borrows: the packet iterator borrows the demuxer while
            // the decoder is used through a disjoint field.
            let this = &mut *self;
            let Self { ictx, decoder, scaler, stream_index, time_base_s,
                       sent_eof, finished, .. } = this;
            loop {
                let mut frame = ffmpeg::util::frame::Video::empty();
                if decoder.receive_frame(&mut frame).is_ok() {
                    if let Some(f) = convert(scaler, &frame, *time_base_s) {
                        return Some(f);
                    }
                    continue; // unusable frame: skip to the next one
                }
                if *sent_eof {
                    *finished = true;
                    return None;
                }
                // Feed the next packet belonging to our stream. Corrupt
                // packets are skipped; the demuxer always advances, so this
                // loop is guaranteed to terminate.
                let mut fed = false;
                for item in ictx.packets() {
                    match item {
                        Ok((stream, packet)) if stream.index() == *stream_index => {
                            let _ = decoder.send_packet(&packet);
                            fed = true;
                            break;
                        }
                        Ok(_) => {}
                        Err(_) => {}
                    }
                }
                if !fed {
                    *sent_eof = true;
                    let _ = decoder.send_eof();
                }
            }
        }
    }

    fn convert(
        scaler: &mut Option<ffmpeg::software::scaling::Context>,
        frame: &ffmpeg::util::frame::Video,
        time_base_s: f64,
    ) -> Option<VideoFrame> {
        let (fw, fh) = (frame.width(), frame.height());
        if fw == 0 || fh == 0 || fw > 8192 || fh > 8192 { return None; }

        // The scaler is built lazily from the first frame's actual format.
        if scaler.is_none() {
            *scaler = ffmpeg::software::scaling::Context::get(
                frame.format(), fw as i32, fh as i32,
                ffmpeg::util::format::pixel::Pixel::BGRA, fw as i32, fh as i32,
                ffmpeg::software::scaling::Flags::BILINEAR,
            ).ok()?;
        }
        let mut rgb = ffmpeg::util::frame::Video::empty();
        scaler.as_mut()?.run(frame, &mut rgb).ok()?;

        let (w, h) = (rgb.width(), rgb.height());
        let stride = rgb.stride(0);
        let data = rgb.data(0);
        if w == 0 || h == 0 { return None; }
        let need = stride.checked_mul(h as usize)?;
        if data.len() < need { return None; }

        // BGRA bytes (B,G,R,A) read little-endian are exactly 0xAARRGGBB.
        let mut pixels = vec![0u32; w as usize * h as usize];
        for y in 0..h as usize {
            let row = &data[y * stride .. y * stride + w as usize * 4];
            for (x, c) in row.chunks_exact(4).enumerate() {
                pixels[y * w as usize + x] = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
            }
        }
        let pts = frame.timestamp()
            .filter(|_| time_base_s > 0.0)
            .map(|t| t as f64 * time_base_s);
        Some(VideoFrame { width: w, height: h, pixels, pts_seconds: pts })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_mitosv(path: &std::path::PathBuf) {
        let (w, h, fps, frames) = (4u32, 2u32, 2u32, 2u32);
        let mut v = Vec::new();
        v.extend_from_slice(&format::MITOSV_MAGIC);
        v.extend_from_slice(&w.to_le_bytes());
        v.extend_from_slice(&h.to_le_bytes());
        v.extend_from_slice(&fps.to_le_bytes());
        v.extend_from_slice(&frames.to_le_bytes());
        v.extend_from_slice(&format::FORMAT_RGBA_LE.to_le_bytes());
        for f in 0..frames {
            for _ in 0..w * h {
                v.extend_from_slice(&[f as u8, 0x40, 0x80, 0xFF]); // R,G,B,A
            }
        }
        std::fs::write(path, v).unwrap();
    }

    #[test]
    fn mitosv_round_trip() {
        let path = std::env::temp_dir().join(format!(
            "mitos-boot-test-{}-{:?}.mitosv", std::process::id(), std::time::SystemTime::now()));
        write_mitosv(&path);
        let cfg = VideoSection::default();
        let mut d = open(&path, &cfg).unwrap();
        assert_eq!(d.info().width, 4);
        assert_eq!(d.info().frame_count, Some(2));
        let f0 = d.next_frame().unwrap();
        assert_eq!(f0.pts_seconds, Some(0.0));
        // First pixel is R=0,G=0x40,B=0x80,A=0xFF → 0xFF804000.
        assert_eq!(f0.pixels[0], 0xFF004080);
        let f1 = d.next_frame().unwrap();
        assert_eq!(f1.pts_seconds, Some(0.5));
        assert!(d.next_frame().is_none());
        assert!(d.finished());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn rejects_oversize_asset() {
        let path = std::env::temp_dir().join(format!("mitos-boot-big-{}.mitosv", std::process::id()));
        std::fs::write(&path, vec![0u8; 64]).unwrap();
        let mut cfg = VideoSection::default();
        cfg.max_asset_bytes = 16;
        assert!(open(&path, &cfg).is_err());
        let _ = std::fs::remove_file(&path);
    }

    // ---- format 1: paletted, zlib-compressed index deltas ---------------

    fn zlib(data: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    /// 4x2, 2 fps, 2 frames, palette {black, red, blue}.
    ///   frame 0 indices: 1 1 2 2 / 0 0 1 1
    ///   frame 1 indices: 1 1 2 2 / 2 2 1 1
    fn write_pal8z(path: &std::path::PathBuf, palette_len: u16) {
        let mut v = Vec::new();
        v.extend_from_slice(&format::MITOSV_MAGIC);
        for n in [4u32, 2, 2, 2, format::FORMAT_PAL8_ZDELTA] {
            v.extend_from_slice(&n.to_le_bytes());
        }
        v.extend_from_slice(&palette_len.to_le_bytes());
        v.extend_from_slice(&[0, 0, 0, 255, 0, 0, 0, 0, 255]);
        for delta in [[1u8, 1, 2, 2, 0, 0, 1, 1], [0, 0, 0, 0, 2, 2, 0, 0]] {
            let z = zlib(&delta);
            v.extend_from_slice(&(z.len() as u32).to_le_bytes());
            v.extend_from_slice(&z);
        }
        std::fs::write(path, v).unwrap();
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("mitos-boot-{name}-{}.mitosv", std::process::id()))
    }

    #[test]
    fn pal8z_round_trip_and_rewind() {
        let path = tmp("pal8z");
        write_pal8z(&path, 3);
        let mut d = open(&path, &VideoSection::default()).unwrap();
        assert_eq!((d.info().width, d.info().height), (4, 2));
        let (r, b, k) = (0xFFFF0000u32, 0xFF0000FFu32, 0xFF000000u32);

        let f0 = d.next_frame().unwrap();
        assert_eq!(f0.pixels, vec![r, r, b, b, k, k, r, r]);
        let f1 = d.next_frame().unwrap();
        assert_eq!(f1.pixels, vec![r, r, b, b, b, b, r, r]);
        assert_eq!(f1.pts_seconds, Some(0.5));
        assert!(d.next_frame().is_none());
        assert!(d.finished());

        // Loop playback restarts from the keyframe, not from the last frame.
        assert!(d.rewind());
        assert!(!d.finished());
        let again = d.next_frame().unwrap();
        assert_eq!(again.pixels, f0.pixels);
        assert_eq!(again.pts_seconds, Some(0.0));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn pal8z_truncated_chunk_stops_cleanly() {
        let path = tmp("pal8z-trunc");
        write_pal8z(&path, 3);
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.truncate(bytes.len() - 3); // cut into the last chunk
        std::fs::write(&path, bytes).unwrap();
        let mut d = open(&path, &VideoSection::default()).unwrap();
        assert!(d.next_frame().is_some());
        assert!(d.next_frame().is_none());
        assert!(d.finished());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn pal8z_rejects_empty_palette() {
        let path = tmp("pal8z-nopal");
        write_pal8z(&path, 0);
        assert!(open(&path, &VideoSection::default()).is_err());
        let _ = std::fs::remove_file(&path);
    }

    /// The shipped boot asset must decode end to end with the real decoder.
    #[test]
    fn shipped_owl_asset_decodes() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/splash/owl/owl.mitosv");
        if !path.exists() { return; }
        let mut d = open(&path, &VideoSection::default()).unwrap();
        let (w, h) = (d.info().width, d.info().height);
        let mut n = 0u64;
        while let Some(f) = d.next_frame() {
            assert!(f.dimensions_valid());
            assert_eq!((f.width, f.height), (w, h));
            n += 1;
        }
        assert_eq!(Some(n), d.info().frame_count);
        assert!(d.finished());
        assert!(d.rewind());
        assert!(d.next_frame().is_some());
    }
}
