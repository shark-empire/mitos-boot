#!/usr/bin/env bash
# Launch QEMU with the MITOS test initramfs (design doc §34).
#
#   scripts/test-qemu.sh                      # DRM via virtio-gpu
#   BOOT=fbdev scripts/test-qemu.sh           # fbdev via -vga std
#   KERNEL=/path/vmlinuz scripts/test-qemu.sh
#   APPEND="mitos-boot.debug=1" scripts/test-qemu.sh
#
# Kernel needs: CONFIG_DRM_VIRTIO_GPU (drm mode) or CONFIG_FB + simpledrm/vesa
# (fbdev mode). Serial console shows the boot log.
set -euo pipefail
cd "$(dirname "$0")/.."

KERNEL="${KERNEL:-/boot/vmlinuz}"
INITRD="build/mitos-initramfs.cpio.gz"
BACKEND="${BOOT:-drm}"

[ -f "$KERNEL" ] || { echo "error: kernel not found: $KERNEL (set KERNEL=...)"; exit 1; }
[ -f "$INITRD" ] || { echo "error: $INITRD missing (run scripts/build-initramfs.sh)"; exit 1; }

CMDLINE="console=ttyS0 ${APPEND:-}"

GPU_ARGS=()
case "$BACKEND" in
    drm)
        CMDLINE="$CMDLINE mitos-boot.backend=drm"
        GPU_ARGS+=(-device virtio-gpu-pci -device virtio-keyboard-pci)
        ;;
    fbdev)
        CMDLINE="$CMDLINE mitos-boot.backend=fbdev"
        GPU_ARGS+=(-vga std)
        ;;
    *) echo "error: BOOT must be 'drm' or 'fbdev'"; exit 1 ;;
esac

KVM=()
if [ -w /dev/kvm ]; then KVM+=(-enable-kvm -cpu host); fi

exec qemu-system-x86_64 \
    -machine q35 -m 512M -nodefaults \
    ${KVM[@]+"${KVM[@]}"} \
    -kernel "$KERNEL" -initrd "$INITRD" -append "$CMDLINE" \
    ${GPU_ARGS[@]+"${GPU_ARGS[@]}"} \
    -serial stdio -monitor none \
    -display gtk,show-cursor=on