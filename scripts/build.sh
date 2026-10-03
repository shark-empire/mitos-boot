#!/usr/bin/env bash
# Build mitos-boot.
#   MITOS_BOOT_FEATURES=""           dependency-free build (MITOSV assets only)
#   MITOS_BOOT_FEATURES=video-ffmpeg full build (default)
set -euo pipefail
cd "$(dirname "$0")/.."

FEATURES="${MITOS_BOOT_FEATURES-video-ffmpeg}"

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