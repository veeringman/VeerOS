#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────────
# VeerOS — ESP32-C3 Build, Flash & Monitor Script
# ──────────────────────────────────────────────────────────────────────────
#
# Builds the kernel-esp32c3 crate, flashes it to the connected
# ESP32-C3 via USB, and optionally opens a serial monitor.
#
# Usage:
#   ./scripts/build-esp32c3.sh                   # build + flash + monitor
#   ./scripts/build-esp32c3.sh --build           # build only
#   ./scripts/build-esp32c3.sh --flash           # flash only (skip build)
#   ./scripts/build-esp32c3.sh --monitor         # monitor only
#   ./scripts/build-esp32c3.sh --dist minimal    # minimal distribution
#   ./scripts/build-esp32c3.sh --dist app        # app distribution
#   ./scripts/build-esp32c3.sh --dist full       # full (app + rt)
#   ./scripts/build-esp32c3.sh --port /dev/X     # override serial port
#   ./scripts/build-esp32c3.sh --baud 921600     # override flash baud rate
#   ./scripts/build-esp32c3.sh --clean           # cargo clean first
#   ./scripts/build-esp32c3.sh --no-monitor      # build + flash, skip monitor
#
# Requirements:
#   - Rust toolchain with riscv32imc-unknown-none-elf target
#   - espflash (cargo install espflash)
#   - ESP32-C3 connected via USB data cable
#
set -euo pipefail

# ── Paths ─────────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
TARGET=riscv32imc-unknown-none-elf
PKG=kernel-esp32c3
RELEASE_DIR="$ROOT_DIR/dist/esp32c3/release"
PARTITION_TABLE="$ROOT_DIR/crates/kernel/esp32c3/partitions.csv"

# ── Defaults ──────────────────────────────────────────────────────────────

DIST="minimal"
PROFILE="release"
DO_CLEAN=false
DO_BUILD=true
DO_FLASH=true
DO_MONITOR=true
PORT=""
BAUD=921600
MONITOR_BAUD=115200
WIFI_SSID=""
WIFI_PASS=""

# ── Parse arguments ───────────────────────────────────────────────────────

while [[ $# -gt 0 ]]; do
    case "$1" in
        --dist)       DIST="$2";          shift 2 ;;
        --debug)      PROFILE="dev";      shift ;;
        --clean)      DO_CLEAN=true;      shift ;;
        --build)      DO_BUILD=true; DO_FLASH=false; DO_MONITOR=false; shift ;;
        --flash)      DO_BUILD=false; DO_FLASH=true; DO_MONITOR=true; shift ;;
        --monitor)    DO_BUILD=false; DO_FLASH=false; DO_MONITOR=true; shift ;;
        --no-monitor) DO_MONITOR=false;   shift ;;
        --port)       PORT="$2";          shift 2 ;;
        --baud)       BAUD="$2";          shift 2 ;;
        --ssid)       WIFI_SSID="$2";     shift 2 ;;
        --password)   WIFI_PASS="$2";     shift 2 ;;
        -h|--help)
            sed -n '2,/^set -/p' "$0" | grep '^#' | sed 's/^# \?//'
            exit 0 ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# ── Map distribution name to Cargo features ──────────────────────────────

case "$DIST" in
    minimal) FEATURES="dist-minimal,shell,wifi,ble" ;;
    app)     FEATURES="dist-app" ;;
    rt)      FEATURES="dist-rt,shell,wifi,ble" ;;
    full)    FEATURES="dist-full" ;;
    *)       echo "Unknown dist: $DIST (use minimal|app|rt|full)"; exit 1 ;;
esac

# ── Auto-detect serial port ──────────────────────────────────────────────

if [[ -z "$PORT" ]]; then
    # Look for Espressif USB JTAG/serial debug unit.
    for dev in /dev/ttyACM* /dev/ttyUSB*; do
        if [[ -e "$dev" ]]; then
            PORT="$dev"
            break
        fi
    done
    if [[ -z "$PORT" ]]; then
        echo "ERROR: No ESP32 serial device found."
        echo "  • Ensure the ESP32-C3 is connected with a USB data cable"
        echo "  • Check: ls /dev/ttyACM* /dev/ttyUSB*"
        exit 1
    fi
fi

echo "──────────────────────────────────────────────────"
echo "  VeerOS ESP32-C3 Build System"
echo "──────────────────────────────────────────────────"
echo "  Distribution : $DIST"
echo "  Profile      : $PROFILE"
echo "  Target       : $TARGET"
echo "  Serial port  : $PORT"
echo "  Flash baud   : $BAUD"
if [[ -n "$WIFI_SSID" ]]; then
    echo "  WiFi SSID    : $WIFI_SSID"
fi
echo "  Actions      : $(${DO_BUILD} && echo 'build')$(${DO_FLASH} && echo ' flash')$(${DO_MONITOR} && echo ' monitor')"
echo "──────────────────────────────────────────────────"
echo ""

# ── Build ─────────────────────────────────────────────────────────────────

ELF_PATH="$ROOT_DIR/target/$TARGET/release/$PKG"

if $DO_BUILD; then
    if $DO_CLEAN; then
        echo "[build] cargo clean..."
        cargo clean --manifest-path "$ROOT_DIR/Cargo.toml" 2>/dev/null || true
    fi

    echo "[build] compiling $PKG ($DIST, $PROFILE)..."

    # Pass WiFi credentials as env vars for compile-time embedding.
    if [[ -n "$WIFI_SSID" ]]; then
        export VEEROS_WIFI_SSID="$WIFI_SSID"
    fi
    if [[ -n "$WIFI_PASS" ]]; then
        export VEEROS_WIFI_PASS="$WIFI_PASS"
    fi

    CARGO_ARGS=(
        build
        --manifest-path "$ROOT_DIR/Cargo.toml"
        --target "$TARGET"
        -p "$PKG"
        --no-default-features
        --features "$FEATURES"
    )
    if [[ "$PROFILE" == "release" ]]; then
        CARGO_ARGS+=( --release )
    fi

    cargo "${CARGO_ARGS[@]}"

    # ── Copy artifacts to dist/ ───────────────────────────────────────
    mkdir -p "$RELEASE_DIR"
    cp "$ELF_PATH" "$RELEASE_DIR/$PKG.elf"

    # Report sizes.
    echo ""
    echo "[build] ELF size:"
    size "$ELF_PATH" 2>/dev/null || ls -la "$ELF_PATH"
    echo ""
    echo "[build] build complete: $RELEASE_DIR/$PKG.elf"
fi

# ── Flash ─────────────────────────────────────────────────────────────────

if $DO_FLASH; then
    if [[ ! -f "$ELF_PATH" ]]; then
        echo "ERROR: ELF not found at $ELF_PATH"
        echo "  Run with --build first, or without --flash to build."
        exit 1
    fi

    echo ""
    echo "[flash] flashing $PKG to $PORT @ ${BAUD} baud..."
    echo "[flash] chip: ESP32-C3"
    echo ""

    # Release any existing monitor holding the port.
    kill $(lsof -t "$PORT") 2>/dev/null || true
    sleep 1

    FLASH_ARGS=(
        flash
        --chip esp32c3
        --port "$PORT"
        --baud "$BAUD"
        --partition-table "$PARTITION_TABLE"
        --ignore-app-descriptor
    )

    espflash "${FLASH_ARGS[@]}" "$ELF_PATH"

    echo "[flash] done!"

    # Brief pause so the chip finishes reset and boot output starts.
    sleep 2
fi

# ── Monitor ───────────────────────────────────────────────────────────────

if $DO_MONITOR; then
    echo ""
    echo "[monitor] opening serial monitor on $PORT ..."
    echo "[monitor] (USB Serial JTAG — baud rate is ignored by CDC-ACM)"
    echo "[monitor] press Ctrl+A then Ctrl+X to exit"
    echo ""

    # Kill any stale monitor still holding the port.
    kill $(lsof -t "$PORT") 2>/dev/null || true
    sleep 0.5

    # Use picocom for USB Serial JTAG.  The baud setting is cosmetic
    # (CDC-ACM ignores it) but picocom requires one.
    # --omap lfcrlf: convert \n from the device into \r\n for the terminal.
    exec picocom -b "$MONITOR_BAUD" --omap lfcrlf "$PORT"
fi
