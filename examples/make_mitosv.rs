//! Generate a synthetic MITOSV test animation — lets you exercise the full
//! video pipeline (decoder → player → renderer → display) with zero FFmpeg
//! and zero artistic assets.
//!
//!   cargo run --release --example make_mitosv -- out.webm 320 240 60 30
//!
//! (Name it .webm and it drops straight into the default config path; the
//! decoder identifies assets by magic bytes, not extension.)

use std::fs::File;
use std::io::{BufWriter, Write};

const MAX_DIM: u32 = 8192;
const MAX_FPS: u32 = 240;
const MAX_FRAMES: u32 = 100_000;

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| "out.webm".to_string());
    let w: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(320);
    let h: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(240);
    let frames: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(60);
    let fps: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(30);

    // Mirror the decoder's validation so the tool never emits a rejected file.
    for (v, hi, what) in [(w, MAX_DIM, "width"), (h, MAX_DIM, "height")] {
        if v == 0 || v > hi { eprintln!("error: {what} must be 1..={hi}"); std::process::exit(2); }
    }
    if fps == 0 || fps > MAX_FPS { eprintln!("error: fps must be 1..={MAX_FPS}"); std::process::exit(2); }
    if frames == 0 || frames > MAX_FRAMES { eprintln!("error: frames must be 1..={MAX_FRAMES}"); std::process::exit(2); }

    let mut f = BufWriter::new(File::create(&out).expect("create output"));
    f.write_all(b"MITOSV1\0").unwrap();
    f.write_all(&w.to_le_bytes()).unwrap();
    f.write_all(&h.to_le_bytes()).unwrap();
    f.write_all(&fps.to_le_bytes()).unwrap();
    f.write_all(&frames.to_le_bytes()).unwrap();
    f.write_all(&0u32.to_le_bytes()).unwrap(); // FORMAT_RGBA_LE

    let mut row = Vec::with_capacity((w as usize) * 4);
    for i in 0..frames {
        let t = i as f32 / frames as f32;
        // A drifting, pulsing glow: fades in, drifts right, fades out.
        let cx = w as f32 * (0.30 + 0.40 * t);
        let cy = h as f32 * 0.5;
        let fade = (t * std::f32::consts::PI).sin().max(0.0);
        for y in 0..h {
            row.clear();
            for x in 0..w {
                let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
                let v = (1.0 - d / (w.min(h) as f32 * 0.45)).clamp(0.0, 1.0);
                row.extend_from_slice(&[
                    (v * 122.0 * fade) as u8, // R
                    (v * 168.0 * fade) as u8, // G
                    (v * 255.0 * fade) as u8, // B
                    0xFF,                     // A
                ]);
            }
            f.write_all(&row).unwrap();
        }
    }
    f.flush().unwrap();
    println!("wrote {out}: {w}x{h}, {frames} frames @ {fps} fps");
}