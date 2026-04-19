#!/usr/bin/env bash
#
# Build the VeerOS QEMU-emulated ESP32-C6 kernel (RISC-V 32-bit).
#
# This kernel runs on QEMU virt machine with ESP32-C6 personality:
#   - 512 KiB RAM (matching ESP32-C6 SRAM)
#   - Simulated WiFi / BLE / 802.15.4
#   - Virtual sensor subsystem for EdgeFabric integration
#
# Usage:
#   ./scripts/build-qemu-esp32c6.sh            # debug build
#   ./scripts/build-qemu-esp32c6.sh release     # release build
#
# Run:
#   qemu-system-riscv32 -M virt -m 128M -nographic \
#     -bios none -kernel target/riscv32imc-unknown-none-elf/release/kernel-qemu-esp32c6
#
# With networking:
#   qemu-system-riscv32 -M virt -m 128M -nographic -bios none \
#     -kernel target/riscv32imc-unknown-none-elf/release/kernel-qemu-esp32c6 \
#     -netdev user,id=n0,hostfwd=tcp::2323-:2323 \
#     -device virtio-net-device,netdev=n0

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROFILE="${1:-debug}"
TARGET="riscv32imc-unknown-none-elf"

if [ "$PROFILE" = "release" ]; then
    cargo build --release -p kernel-qemu-esp32c6 --target "$TARGET"
    BINARY="$ROOT/target/$TARGET/release/kernel-qemu-esp32c6"
else
    cargo build -p kernel-qemu-esp32c6 --target "$TARGET"
    BINARY="$ROOT/target/$TARGET/debug/kernel-qemu-esp32c6"
fi

SIZE=$(stat -c%s "$BINARY" 2>/dev/null || stat -f%z "$BINARY")
echo ""
echo "Built: $BINARY"
echo "Size:  $SIZE bytes"
echo ""
echo "Run with:"
echo "  qemu-system-riscv32 -M virt -m 128M -nographic -bios none -kernel $BINARY"
