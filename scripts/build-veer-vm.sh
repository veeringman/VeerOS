#!/usr/bin/env bash
#
# Build the VeerOS x86-64 kernel + veer-vm VMM for the veer-vm/fold launch path.
#
# Unlike `build-qemu-pc.sh`, this script does NOT build a GRUB ISO —
# veer-vm loads the kernel ELF directly via its Multiboot v1 header.
#
# Usage:
#   ./scripts/build-veer-vm.sh            # debug builds
#   ./scripts/build-veer-vm.sh release    # release builds
#
# Run with:
#   ./scripts/veer-vm-net-smoke.sh
#   # or manually:
#   sudo ip tuntap add dev tap0 mode tap user $USER
#   sudo ip addr add 10.0.2.2/24 dev tap0 && sudo ip link set tap0 up
#   ./target/release/veer-vm \
#       --kernel target/x86_64-unknown-none/release/kernel-qemu-pc \
#       --memory 128 --tap tap0

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROFILE="${1:-debug}"

# See build-qemu-pc.sh for the rationale: rustc's stock
# x86_64-unknown-none `core.rlib` is built with code-model=kernel while
# the VeerOS kernel uses code-model=small. Rebuild core with our flags.
export RUSTC_BOOTSTRAP=1
BUILD_STD=(-Z build-std=core,compiler_builtins -Z build-std-features=compiler-builtins-mem)

if [ "$PROFILE" = "release" ]; then
    cargo build --release -p kernel-qemu-pc --target x86_64-unknown-none "${BUILD_STD[@]}"
    cargo build --release -p veer_vm
    KERNEL="$ROOT/target/x86_64-unknown-none/release/kernel-qemu-pc"
    KERNEL_TARGET_ALIAS="$ROOT/target/x86_64-unknown-none/release/kernel-x86_64-pc"
    VMM="$ROOT/target/release/veer-vm"
else
    cargo build -p kernel-qemu-pc --target x86_64-unknown-none "${BUILD_STD[@]}"
    cargo build -p veer_vm
    KERNEL="$ROOT/target/x86_64-unknown-none/debug/kernel-qemu-pc"
    KERNEL_TARGET_ALIAS="$ROOT/target/x86_64-unknown-none/debug/kernel-x86_64-pc"
    VMM="$ROOT/target/debug/veer-vm"
fi

# Also provide a target/ alias without "qemu" in the filename.
cp "$KERNEL" "$KERNEL_TARGET_ALIAS"

# Provide a neutral artifact name for veer-vm/fold workflows.
KERNEL_ALIAS_DIR="$ROOT/build/veer-vm"
mkdir -p "$KERNEL_ALIAS_DIR"
if [ "$PROFILE" = "release" ]; then
    KERNEL_ALIAS="$KERNEL_ALIAS_DIR/kernel-x86_64-release.elf"
else
    KERNEL_ALIAS="$KERNEL_ALIAS_DIR/kernel-x86_64-debug.elf"
fi
cp "$KERNEL" "$KERNEL_ALIAS"

echo ""
echo "Built kernel: $KERNEL"
echo "Kernel target alias: $KERNEL_TARGET_ALIAS"
echo "Kernel alias: $KERNEL_ALIAS"
echo "Built VMM:    $VMM"
echo ""
echo "Smoke-test with:"
echo "  ./scripts/veer-vm-net-smoke.sh"
echo ""
echo "Or run directly (requires tap0 pre-configured on the host):"
echo "  $VMM --kernel $KERNEL_ALIAS --memory 128 --tap tap0"
