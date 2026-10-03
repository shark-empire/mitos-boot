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

# Optional assets: drop owl.webm / owl.png / a font in place if you have them.
for f in assets/splash/owl/owl