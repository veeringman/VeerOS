#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────────
# VeerOS — Download Raspberry Pi firmware boot files
# ──────────────────────────────────────────────────────────────────────────
#
# Downloads the minimal set of RPi firmware files needed to boot the Pi 5.
# These go on the FAT32 boot partition alongside kernel8.img.
#
# Usage:
#   ./scripts/fetch-rpi-firmware.sh
#
# Output: dist/raspi5/firmware/
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
FW_DIR="$ROOT_DIR/dist/raspi5/firmware"

# The RPi firmware repo raw URL
BASE_URL="https://github.com/raspberrypi/firmware/raw/master/boot"

# Minimal files needed to boot a Pi 5 in AArch64 bare-metal mode
FILES=(
    "start4.elf"
    "fixup4.dat"
    "bcm2712-rpi-5-b.dtb"
    "bootcode.bin"
)

mkdir -p "$FW_DIR"

echo "Downloading RPi5 firmware files to dist/raspi5/firmware/ ..."
echo ""

for f in "${FILES[@]}"; do
    if [[ -f "$FW_DIR/$f" ]]; then
        echo "  [skip] $f (already exists)"
    else
        echo "  [get]  $f ..."
        curl -fSL --retry 3 -o "$FW_DIR/$f" "$BASE_URL/$f"
    fi
done

echo ""
echo "Done. Firmware files are in: dist/raspi5/firmware/"
echo "These will be included automatically by deploy-sdcard.sh --device."
