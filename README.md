# mitos-boot

![./assets/splash/owl/owl.mp4]

mitos-boot

MITOS early boot & visual startup system. It shows MITOS starting; it doesnot make it running (mitos-init), create sessions (mitos-session),authenticate (mitos-login), or run the desktop (mitos-gui).

power → firmware → bootloader → Linux kernel → early userspace →mitos-boot → [ display → owl video → MĨȚǑŠ ] → readiness → mitos-init
Sequence

Black → owl video (owl.mitosv: the owl's eyes open, blink, open — see "Owl asset" below) →owl fades → MĨȚǑŠ fades in → hold until the system is actually ready →MĨȚǑŠ fades → handoff. The owl replays (owl_loop = true) for as long as the system needs, like the Apple logo on a Mac, and hands over the moment the system is ready (after the minimum visual duration). The animation never gates readiness and readinessnever crashes the animation: a broken video falls back to a static image,then to a wordmark-only splash, and boot always continues.

Build

scripts/build.sh                       # default build: dependency-light, plays the owl (MITOSV)
MITOS_BOOT_FEATURES=video-ffmpeg scripts/build.sh   # also decode WebM/MP4/... via system FFmpeg
FFmpeg note: only needed for other video containers. ffmpeg-next must match your system libavcodec major version (6.x crate ↔ FFmpeg 6, etc.). The default build has no multimedia dependencies at all — the owl ships as MITOSV format 1 (about 400 KB, decoded with two byte planes of RAM). Generate test animations with cargo run --release --example make_mitosv.

Run

mitos-boot [--config PATH] [--no-splash] [--debug] [--recovery] [--version]
Kernel command line overrides: mitos-boot.splash=0|1, mitos-boot.debug=1,mitos-boot.log=debug, mitos-boot.backend=drm|fbdev|auto,mitos-boot.recovery=1, mitos-boot.headless=1.

Exit codes: 0 success · 78 configuration error · 86 watchdog stall ·1 other fatal (unless the recovery path execs a recovery/console program,in which case that program's exit code becomes ours).

Assets

Purpose	Primary	Fallbacks
Owl	owl.mitosv (MITOSV; any FFmpeg container with --features video-ffmpeg)	owl.png → wordmark-only
Wordmark	rendered from font (full Unicode, MĨȚǑŠ)	built-in ASCII bitmap glyphs
mitos.svg remains a design reference; the wordmark is renderedtypographically so no SVG parser (and its dependency tree) ships in earlyuserspace.

Owl asset (assets/splash/owl/)

owl.mp4 is the source clip. owl.mitosv is what boots: scripts/make-owl-asset.py crops it to the face band (the clip is mostly black margin), maps it onto one 256-colour palette, keeps only real motion (a pixel changes only when it differs from what is on screen by more than --hold, so H.264 shimmer is dropped), and stores each frame as zlib-compressed index deltas. Result: 704x432, 168 frames @ 24 fps, about 400 KB, about 43.7 dB PSNR against the source, and the decoder needs roughly 600 KiB of byte planes plus one frame of pixels instead of FFmpeg.

Rebuild it (needs ffmpeg + python3 + numpy; dev/CI only, nothing here ships in the initramfs):

python3 scripts/make-owl-asset.py assets/splash/owl/owl.mp4 assets/splash/owl/owl.mitosv

The script decodes its own output the way src/video/decoder.rs does and refuses to write a file that does not round-trip bit-exactly. The asset has an opaque black background, so keep background.style = "solid" with black (the default) or the crop edges can show on a gradient.

The format itself (header, palette, chunks) is documented in src/video/format.rs.

Security model (§31 of the design document)

Asset paths must resolve (through symlinks) inside security.allowed_asset_roots.
All reads are size-capped (config file, fonts, PNGs, videos, mountinfo, IPC frames).
IPC: Unix socket, mode 0600, SO_PEERCRED uid check, length-prefixed binaryframes, bounded clients, UTF-8-validated strings, socket unlinked on exit.
execve only on absolute, regular, executable, non-world-writable paths(mitos-init, recovery, console). No shells in the normal boot path.
Every fd is O_CLOEXEC; DRM master is dropped before handoff.
Watchdog: if the boot loop stalls, exit 86 so the surrounding init reacts.
Every vsync/read/poll wait is bounded; nothing can block boot indefinitely.
Testing

See tests/README.md for the staged bring-up plan (Stage 1 black screen →Stage 8 real hardware) and the per-module unit-test map. QEMU:

scripts/build-initramfs.shscripts/test-qemu.sh              # or: BOOT=fbdev scripts/test-qemu.sh
Layout

src/boot.rs        boot coordinator + watchdog (what happens next)src/state.rs       shared runtime state, signal handlingsrc/config.rs      config load/validate/clamp, asset allowlistsrc/display/       DRM/KMS primary, fbdev fallbacksrc/renderer/      surfaces, compositing, text, effectssrc/video/         player + decoder trait (MITOSV raw, FFmpeg optional)src/splash/        owl, wordmark, backgroundsrc/animation/     timeline, transitions, easingsrc/bootlog/       logger (stderr + /dev/kmsg + on-screen ring), progresssrc/system/        readiness monitor (mounts/devices/files/init marker)src/ipc/           boot ⇄ init protocolsrc/handoff.rs     clean exit / exec of mitos-initsrc/error/         error screen + recovery (bitmap-font fallback path)
