#!/usr/bin/env bash
# Install mitos-boot + assets into a target rootfs.
#   DESTDIR=/mnt/target scripts/install.sh
# Existing /etc/mitos/boot.toml is preserved (new config staged as .new).
set -euo pipefail
cd "$(dirname "$0")/.."
DEST="${DESTDIR:-}"

[ -x target/release/mitos-boot ] || { echo "error: run scripts/build.sh first"; exit 1; }

install -D -m 0755 target/release/mitos-boot "$DEST/usr/lib/mitos/mitos-boot"

if [ -f "$DEST/etc/mitos/boot.toml" ]; then
    install -D -m 0644 config/boot.toml "$DEST/etc/mitos/boot.toml.new"
    echo "note: kept existing /etc/mitos/boot.toml (staged boot.toml.new)"
else
    install -D -m 0644 config/boot.toml "$DEST/etc/mitos/boot.toml"
fi

for f in assets/splash/owl/owl.webm assets/splash/owl/owl.png assets/splash/owl/owl.mitosv; do
    [ -f "$f" ] && install -D -m 0644 "$f" "$DEST/usr/share/mitos/boot/$(basename "$f")"
done

if compgen -G "assets/fonts/*.ttf" > /dev/null; then
    for f in assets/fonts/*.ttf; do
        install -D -m 0644 "$f" "$DEST/usr/share/mitos/fonts/$(basename "$f")"
    done
fi

install -d -m 0755 "$DEST/usr/share/mitos/themes"
[ -f assets/themes/default.toml ] && install -m 0644 assets/themes/default.toml \
    "$DEST/usr/share/mitos/themes/default.toml"

echo "==> installed under ${DEST:-/}"