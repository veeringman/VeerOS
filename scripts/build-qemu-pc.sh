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

# The stock `x86_64-unknown-none` target in rustc >=1.95 ships `core.rlib`
# built with `code-model=kernel` (high-half kernel). VeerOS is loaded by
# Multiboot at 1 MiB (low 2 GiB) and is compiled with `code-model=small`.
# LLVM now hard-errors on module-flag mismatches, so we must rebuild
# `core` with our rustflags via `-Z build-std`. This is a nightly-only
# flag, so we set `RUSTC_BOOTSTRAP=1` to enable it on the stable toolchain
# pinned in `rust-toolchain.toml`.
export RUSTC_BOOTSTRAP=1
BUILD_STD=(-Z build-std=core,compiler_builtins -Z build-std-features=compiler-builtins-mem)

if [ "$PROFILE" = "release" ]; then
    cargo build --release -p kernel-qemu-pc --target x86_64-unknown-none "${BUILD_STD[@]}"
    BINARY="$ROOT/target/x86_64-unknown-none/release/kernel-qemu-pc"
else
    cargo build -p kernel-qemu-pc --target x86_64-unknown-none "${BUILD_STD[@]}"
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
echo ""
echo "Run with networking + SSH port forwarding:"
echo "  ./scripts/run-qemu-pc.sh"
echo ""
echo "SSH is exposed as host port 2222 -> guest 10.0.2.15:22 by default."
