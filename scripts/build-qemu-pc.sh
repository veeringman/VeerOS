#!/usr/bin/env bash
#
# Build the VeerOS x86-64 kernel for QEMU q35/pc.
#
# Because the kernel is a 64-bit ELF with Multiboot v1 header, QEMU's
# -kernel flag cannot load it directly.  We create a GRUB bootable ISO.
#
# Usage:
#   ./scripts/build-qemu-pc.sh            # debug build
#   ./scripts/build-qemu-pc.sh release    # release build
#
# Run with (serial log only):
#   qemu-system-x86_64 -M q35 -m 256M -display none -serial stdio \
#     -cdrom build/veeros.iso
#
# Run with VGA display (interactive shell):
#   qemu-system-x86_64 -M q35 -m 256M -cdrom build/veeros.iso

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROFILE="${1:-debug}"

if [ "$PROFILE" = "release" ]; then
    cargo build --release -p kernel-qemu-pc --target x86_64-unknown-none
    BINARY="$ROOT/target/x86_64-unknown-none/release/kernel-qemu-pc"
else
    cargo build -p kernel-qemu-pc --target x86_64-unknown-none
    BINARY="$ROOT/target/x86_64-unknown-none/debug/kernel-qemu-pc"
fi

# ── Create bootable ISO via GRUB ──
ISO_DIR="$ROOT/build/iso"
ISO="$ROOT/build/veeros.iso"
mkdir -p "$ISO_DIR/boot/grub"
cp "$BINARY" "$ISO_DIR/boot/kernel.elf"
cat > "$ISO_DIR/boot/grub/grub.cfg" << 'GRUBCFG'
set timeout=0
set default=0
menuentry "VeerOS" {
    multiboot /boot/kernel.elf
    boot
}
GRUBCFG
grub-mkrescue -o "$ISO" "$ISO_DIR" 2>/dev/null

echo ""
echo "Built: $BINARY"
echo "ISO:   $ISO"
echo ""
echo "Run with (serial log):"
echo "  qemu-system-x86_64 -M q35 -m 256M -display none -serial stdio -cdrom build/veeros.iso"
echo ""
echo "Run with VGA display (interactive shell):"
echo "  qemu-system-x86_64 -M q35 -m 256M -cdrom build/veeros.iso"
