#!/usr/bin/env bash
#
# Build glue for VeerOS ESP32-C6 (riscv32) image + veer-vm + fold.
#
# This does not boot riscv32 under veer-vm yet (runtime backend pending),
# but it ensures the riscv32 kernel ELF is produced and the host-side tools
# are built with architecture wiring (`--arch riscv32`).
#
# Usage:
#   ./scripts/build-veer-vm-esp32c6.sh
#   ./scripts/build-veer-vm-esp32c6.sh release

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROFILE="${1:-debug}"
TARGET="riscv32imc-unknown-none-elf"

if [[ "$PROFILE" == "release" ]]; then
    cargo build --release -p kernel-fold-esp32c6 --target "$TARGET"
    cargo build --release -p veer_vm -p fold_engine
    KERNEL="$ROOT/target/$TARGET/release/kernel-fold-esp32c6"
    VMM="$ROOT/target/release/veer-vm"
    FOLD="$ROOT/target/release/fold"
else
    cargo build -p kernel-fold-esp32c6 --target "$TARGET"
    cargo build -p veer_vm -p fold_engine
    KERNEL="$ROOT/target/$TARGET/debug/kernel-fold-esp32c6"
    VMM="$ROOT/target/debug/veer-vm"
    FOLD="$ROOT/target/debug/fold"
fi

SIZE=$(stat -c%s "$KERNEL" 2>/dev/null || stat -f%z "$KERNEL")

echo ""
echo "Built kernel: $KERNEL"
echo "Size:         $SIZE bytes"
echo "Built VMM:    $VMM"
echo "Built Fold:   $FOLD"
echo ""
echo "Validate veer-vm image loading path:"
echo "  $VMM --arch riscv32 --kernel $KERNEL --memory 128"
echo ""
echo "Launch via fold (wiring check):"
echo "  $FOLD vm spawn --arch riscv32 --kernel $KERNEL --memory 128 --user-ns --name esp32c6-dev"
