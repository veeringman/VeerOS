#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────────
# VeerOS — RPi5 GDB Debug Helper
# ──────────────────────────────────────────────────────────────────────────
#
# Launches GDB connected to an OpenOCD / JTAG session for RPi5 debugging.
#
# Usage:
#   # First, start OpenOCD in another terminal:
#   openocd -f interface/cmsis-dap.cfg -f target/bcm2712.cfg
#
#   # Then run this script:
#   ./scripts/debug-raspi5.sh
#   ./scripts/debug-raspi5.sh --openocd   # also starts OpenOCD
#
# Requirements:
#   - gdb-multiarch or aarch64-none-elf-gdb
#   - OpenOCD with BCM2712 support (or JTAG probe)
#   - kernel-raspi5.elf (from build-raspi5.sh)
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
ELF="$ROOT_DIR/dist/raspi5/release/kernel-raspi5.elf"

if [[ ! -f "$ELF" ]]; then
    echo "Error: ELF not found at $ELF"
    echo "Run ./scripts/build-raspi5.sh first."
    exit 1
fi

# Find GDB
GDB=""
for candidate in gdb-multiarch aarch64-none-elf-gdb aarch64-linux-gnu-gdb; do
    if command -v "$candidate" &>/dev/null; then
        GDB="$candidate"
        break
    fi
done

if [[ -z "$GDB" ]]; then
    echo "Error: No suitable GDB found."
    echo "Install gdb-multiarch:  sudo apt install gdb-multiarch"
    exit 1
fi

# Optional: start OpenOCD
if [[ "${1:-}" == "--openocd" ]]; then
    echo "Starting OpenOCD in background..."
    openocd \
        -f interface/cmsis-dap.cfg \
        -f target/bcm2712.cfg &
    OPENOCD_PID=$!
    sleep 2
    trap "kill $OPENOCD_PID 2>/dev/null" EXIT
fi

echo "Launching $GDB with $ELF ..."
echo "  target remote :3333   (connect to OpenOCD)"
echo "  monitor reset halt    (reset + halt CPU)"
echo "  load                  (flash ELF symbols)"
echo "  break _start          (set entry breakpoint)"
echo "  continue              (run)"
echo ""

"$GDB" \
    -ex "set confirm off" \
    -ex "file $ELF" \
    -ex "target remote :3333" \
    -ex "monitor reset halt" \
    -ex "load" \
    -ex "break _start" \
    -ex "set print pretty on" \
    "$ELF"
