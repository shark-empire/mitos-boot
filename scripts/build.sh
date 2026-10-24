#!/usr/bin/env bash
# Build mitos-boot.
#   (default)                        dependency-light build (MITOSV assets, incl. the owl)
#   MITOS_BOOT_FEATURES=video-ffmpeg also decode WebM/MP4/... via system FFmpeg
set -euo pipefail
cd "$(dirname "$0")/.."

FEATURES="${MITOS_BOOT_FEATURES-}"

echo "==> cargo build --release (features: ${FEATURES:-none})"
if [ -n "$FEATURES" ]; then
    cargo build --release --features "$FEATURES"
else
    cargo build --release --no-default-features
fi

echo "==> unit tests"
if [ -n "$FEATURES" ]; then
    cargo test --release --features "$FEATURES"
else
    cargo test --release --no-default-features
fi

echo "==> done: target/release/mitos-boot"