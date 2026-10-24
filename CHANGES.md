# mitos-boot: what changed

## Owl boot logo (the feature)
- NEW  assets/splash/owl/owl.mitosv: 398 KB boot asset built from your video (cropped to the eyes, 704x432, 168 frames @ 24 fps)
- NEW  scripts/make-owl-asset.py: rebuilds it from owl.mp4 (needs ffmpeg + python3 + numpy); verifies its own output
- src/video/format.rs, decoder.rs: new MITOSV format 1 (palette + zlib index deltas), rewind support, tests
- src/video/player.rs, src/splash/owl.rs, src/splash/mod.rs, src/boot.rs: owl replays until the system is ready
- src/config.rs, config/boot.toml: new `owl_loop` (default true); default path is now owl.mitosv
- Cargo.toml, scripts/build.sh: flate2 added; FFmpeg is opt-in (`MITOS_BOOT_FEATURES=video-ffmpeg`)
- NEW  .github/workflows/ci.yml: build + test on every push
- README.md, tests/README.md: docs

## Fixes found by reading (no Rust compiler was available, so CI is the judge)
- config.rs: u64 field passed to a u32 clamp closure
- handoff.rs: missing OsStringExt import; closure used `?` but returned a bare value
- bootlog/logger.rs: write to a non-mut &File; /dev/kmsg got one message split into many writes
- display/framebuffer.rs, display/drm.rs: missing OsStrExt import
- display/drm.rs: wrong field name (connector_type -> kind), non-mut map, ModeInfo missing Debug
- display/drm.rs (kernel ABI): SETCRTC ioctl number 0x82 -> 0xa2, drm_mode_card_res field order,
  page-flip struct field order, flip-complete event id 3 -> 2, preferred-mode flag 2 -> 8, musl-safe ioctl cast
- renderer/text.rs: rusttype has pixel_bounding_box(), not bounding_box(); test helper had a type mismatch
- splash/background.rs: explicit u32 literal

## Left alone
- assets/splash/owl/owl is a 1-byte stray file (safe to delete)
- The source clip carries a TikTok watermark (@classicwal). The crop removes it from the boot screen,
  but check you have the right to ship the clip.
