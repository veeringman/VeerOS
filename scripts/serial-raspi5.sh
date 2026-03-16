#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────────
# VeerOS — RPi5 Serial Monitor
# ──────────────────────────────────────────────────────────────────────────
#
# Opens a serial console to the RPi5 UART (115200 baud).
# Connect a USB-to-serial adapter to GPIO14 (TX) and GPIO15 (RX).
#
# Usage:
#   ./scripts/serial-raspi5.sh              # auto-detect /dev/ttyUSB0
#   ./scripts/serial-raspi5.sh /dev/ttyACM0 # explicit device
#   ./scripts/serial-raspi5.sh --baud 9600  # custom baud rate
#
set -euo pipefail

BAUD=115200
DEVICE=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --baud) BAUD="$2"; shift 2 ;;
        /dev/*) DEVICE="$1"; shift ;;
        *)      echo "Usage: $0 [/dev/ttyXXX] [--baud RATE]"; exit 1 ;;
    esac
done

# Auto-detect device
if [[ -z "$DEVICE" ]]; then
    for d in /dev/ttyUSB0 /dev/ttyACM0 /dev/ttyAMA0 /dev/serial0; do
        if [[ -c "$d" ]]; then
            DEVICE="$d"
            break
        fi
    done
fi

if [[ -z "$DEVICE" ]]; then
    echo "Error: No serial device found. Specify explicitly:"
    echo "  $0 /dev/ttyUSB0"
    exit 1
fi

echo "VeerOS Serial Monitor"
echo "  Device : $DEVICE"
echo "  Baud   : $BAUD"
echo "  Exit   : Ctrl-A then X (picocom) / Ctrl-] (minicom)"
echo ""

# Prefer picocom, fall back to minicom, then screen
if command -v picocom &>/dev/null; then
    exec picocom -b "$BAUD" "$DEVICE"
elif command -v minicom &>/dev/null; then
    exec minicom -b "$BAUD" -D "$DEVICE"
elif command -v screen &>/dev/null; then
    exec screen "$DEVICE" "$BAUD"
else
    echo "Error: No serial terminal found. Install one:"
    echo "  sudo apt install picocom   # recommended"
    echo "  sudo apt install minicom"
    echo "  sudo apt install screen"
    exit 1
fi
