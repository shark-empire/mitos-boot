#!/usr/bin/env bash
# Assemble a minimal test initramfs: busybox + mitos-boot + assets + a mock
# readiness signaller. Requires: busybox, cpio.
#   BUSYBOX=/path/to/busybox scripts/build-initramfs.sh [output-dir]
set -euo pipefail
cd "$(dirname "$0")/.."

ROOT="${1:-build/initramfs}"
BUSYBOX="${BUSYBOX:-$(command -v busybox || true)}"

[ -n "$BUSYBOX" ] || { echo "error: busybox not found (set BUSYBOX=...)"; exit 1; }
[ -x target/release/mitos-boot ] || { echo "error: run scripts/build.sh first"; exit 1; }

rm -rf "$ROOT"
install -d -m 0755 "$ROOT"/{bin,dev,proc,sys,run,etc/mitos,tmp,usr/lib/mitos,usr/share/mitos/boot}
install -m 0755 "$BUSYBOX" "$ROOT/bin/busybox"
ln -sf busybox "$ROOT/bin/sh"
install -m 0755 target/release/mitos-boot "$ROOT/usr/lib/mitos/mitos-boot"

# PID 1 must never exit: keep a shell alive under mitos-boot.
cat > "$ROOT/init" <<'EOF'
#!/bin/sh
export PATH=/bin:/usr/bin:/sbin:/usr/sbin
mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev 2>/dev/null || mdev -s
mount -t tmpfs tmpfs /run
mkdir -p /dev/dri /run/mitos
/usr/lib/mitos/mock-init.sh &
/usr/lib/mitos/mitos-boot --config /etc/mitos/boot.toml || /bin/sh
exec /bin/sh
EOF
chmod 0755 "$ROOT/init"

# Mock mitos-init: signals readiness after a few seconds.
cat > "$ROOT/usr/lib/mitos/mock-init.sh" <<'EOF'
#!/bin/sh
sleep 3
touch /run/mitos/system-ready
EOF
chmod 0755 "$ROOT/usr/lib/mitos/mock-init.sh"

# Test configuration: notify-mode handoff, generous timeout, shell recovery.
cat > "$ROOT/etc/mitos/boot.toml" <<'EOF'
[boot]
enabled = true
[system]
readiness_timeout_ms = 8000
init_ready_marker = "/run/mitos/system-ready"
[handoff]
mode = "notify"
[recovery]
console = "/bin/sh"
EOF

# Optional assets: drop owl.webm / owl.png / owl.mitosv in place if you have
# them. The decoder sniffs by magic, so a MITOSV file named owl.webm works.
for f in assets/splash/owl/owl.webm assets/splash/owl/owl.png assets/splash/owl/owl.mitosv; do
    if [ -f "$f" ]; then
        install -m 0644 "$f" "$ROOT/usr/share/mitos/boot/$(basename "$f")"
    fi
done

# Fonts: prefer a bundled MITOS font, else any system TTF (the wordmark's
# diacritics need a real font; without one it degrades to ASCII "MITOS").
FONT_INSTALLED=0
for f in assets/fonts/*.ttf /usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf; do
    if [ -f "$f" ]; then
        install -D -m 0644 "$f" "$ROOT/usr/share/mitos/fonts/MitosDisplay-Regular.ttf"
        FONT_INSTALLED=1
        break
    fi
done
[ "$FONT_INSTALLED" -eq 1 ] || echo "note: no font found; wordmark uses ASCII fallback"

# Early bring-up stages: show diagnostics on screen.
cat >> "$ROOT/etc/mitos/boot.toml" <<'EOF'
[debug]
show_boot_messages = true
log_level = "debug"
EOF

# The kernel opens /dev/console for PID 1 before devtmpfs is mounted.
if [ ! -e "$ROOT/dev/console" ]; then
    mknod -m 0600 "$ROOT/dev/console" c 5 1
fi

mkdir -p build
( cd "$ROOT" && find . -print0 | cpio --null -o --format=newc 2>/dev/null | gzip -9 ) \
    > build/mitos-initramfs.cpio.gz

echo "==> wrote build/mitos-initramfs.cpio.gz ($(du -h build/mitos-initramfs.cpio.gz | cut -f1))"