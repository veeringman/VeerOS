#!/usr/bin/env bash
#
# Build glue for VeerOS ESP32-C6 (riscv32) image + veer-vm + fold.
#
# Builds all three kernel variants and host-side veer-vm/fold tooling:
#   - kernel-qemu-esp32c6   standalone veer-vm path (VirtualIoT / EdgeFabric)
#   - kernel-fold-esp32c6   fold-managed path
#
# On macOS, veer-vm can run `--arch riscv32` via:
#   - in-process rv32-soft (`--riscv32-backend soft`, default), or
#   - external QEMU fallback (`--riscv32-backend qemu`).
#
# Network backends:
#   --net user                  user-net (macOS, no root, recommended for VirtualIoT)
#   --tap tap0                  virtio-net TAP (Linux)
#
# Usage:
#   ./scripts/build-veer-vm-esp32c6.sh
#   ./scripts/build-veer-vm-esp32c6.sh release

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROFILE="${1:-debug}"
TARGET="riscv32imc-unknown-none-elf"

if [[ "$PROFILE" == "release" ]]; then
    cargo build --release \
        -p kernel-qemu-esp32c6 \
        -p kernel-fold-esp32c6 \
        --target "$TARGET"
    cargo build --release -p veer_vm -p fold_engine
    QEMU_KERNEL="$ROOT/target/$TARGET/release/kernel-qemu-esp32c6"
    FOLD_KERNEL="$ROOT/target/$TARGET/release/kernel-fold-esp32c6"
    VMM="$ROOT/target/release/veer-vm"
    FOLD="$ROOT/target/release/fold"
else
    cargo build \
        -p kernel-qemu-esp32c6 \
        -p kernel-fold-esp32c6 \
        --target "$TARGET"
    cargo build -p veer_vm -p fold_engine
    QEMU_KERNEL="$ROOT/target/$TARGET/debug/kernel-qemu-esp32c6"
    FOLD_KERNEL="$ROOT/target/$TARGET/debug/kernel-fold-esp32c6"
    VMM="$ROOT/target/debug/veer-vm"
    FOLD="$ROOT/target/debug/fold"
fi

QEMU_SIZE=$(stat -c%s "$QEMU_KERNEL" 2>/dev/null || stat -f%z "$QEMU_KERNEL")
FOLD_SIZE=$(stat -c%s "$FOLD_KERNEL" 2>/dev/null || stat -f%z "$FOLD_KERNEL")

echo ""
echo "Built kernel (qemu/VirtualIoT): $QEMU_KERNEL  ($QEMU_SIZE bytes)"
echo "Built kernel (fold):            $FOLD_KERNEL  ($FOLD_SIZE bytes)"
echo "Built VMM:    $VMM"
echo "Built Fold:   $FOLD"
echo ""
echo "── VirtualIoT / EdgeFabric (user-net, macOS, no root) ──"
echo "  $VMM --arch riscv32 --kernel $QEMU_KERNEL --memory 32 --net user --net-hostfwd-port 12001"
echo "  # veer-connect: veer-connect shell 127.0.0.1 12001"
echo ""
echo "── Standalone soft-emulation ──"
echo "  $VMM --arch riscv32 --kernel $QEMU_KERNEL --memory 128 --riscv32-backend soft"
echo "  $VMM --arch riscv32 --kernel $QEMU_KERNEL --memory 128 --riscv32-backend qemu"
echo ""
echo "── Launch via fold (wiring check) ──"
echo "  $FOLD vm spawn --arch riscv32 --kernel $FOLD_KERNEL --memory 128 --riscv32-backend soft --user-ns --name esp32c6-dev"
