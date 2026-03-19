#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────────
# VeerOS — XIAO ESP32-C6 Build, Flash & Monitor Script
# ──────────────────────────────────────────────────────────────────────────
#
# Builds the kernel-xiao-esp32c6 crate, flashes it to the connected
# XIAO ESP32-C6 via USB, and optionally opens a serial monitor.
#
# Usage:
#   ./scripts/build-esp32c6.sh                   # build + flash + monitor
#   ./scripts/build-esp32c6.sh --build           # build only
#   ./scripts/build-esp32c6.sh --flash           # flash only (skip build)
#   ./scripts/build-esp32c6.sh --monitor         # monitor only
#   ./scripts/build-esp32c6.sh --dist minimal    # minimal distribution
#   ./scripts/build-esp32c6.sh --dist app        # app distribution
#   ./scripts/build-esp32c6.sh --dist full       # full (app + rt)
#   ./scripts/build-esp32c6.sh --port /dev/X     # override serial port
#   ./scripts/build-esp32c6.sh --baud 921600     # override flash baud rate
#   ./scripts/build-esp32c6.sh --clean           # cargo clean first
#   ./scripts/build-esp32c6.sh --no-monitor      # build + flash, skip monitor
#
# Requirements:
#   - Rust toolchain with riscv32imc-unknown-none-elf target
#   - espflash (cargo install espflash)
#   - XIAO ESP32-C6 connected via USB-C data cable
#
set -euo pipefail

# ── Paths ─────────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
TARGET=riscv32imc-unknown-none-elf
PKG=kernel-xiao-esp32c6
RELEASE_DIR="$ROOT_DIR/dist/esp32c6/release"
PARTITION_TABLE="$ROOT_DIR/crates/kernel/xiao_esp32c6/partitions.csv"

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
        -h|--help)
            sed -n '2,/^set -/p' "$0" | grep '^#' | sed 's/^# \?//'
            exit 0 ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# ── Map distribution name to Cargo features ──────────────────────────────

case "$DIST" in
    minimal) FEATURES="dist-minimal,shell,wifi,ble,ieee802154" ;;
    app)     FEATURES="dist-app" ;;
    rt)      FEATURES="dist-rt,shell,wifi,ble,ieee802154" ;;
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
        echo "  • Ensure the XIAO ESP32-C6 is connected with a USB-C data cable"
        echo "  • Check: ls /dev/ttyACM* /dev/ttyUSB*"
        exit 1
    fi
fi

echo "──────────────────────────────────────────────────"
echo "  VeerOS ESP32-C6 Build System"
echo "──────────────────────────────────────────────────"
echo "  Distribution : $DIST"
echo "  Profile      : $PROFILE"
echo "  Target       : $TARGET"
echo "  Serial port  : $PORT"
echo "  Flash baud   : $BAUD"
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
    echo "[flash] chip: ESP32-C6"
    echo ""

    # Release any existing monitor holding the port.
    kill $(lsof -t "$PORT") 2>/dev/null || true
    sleep 1

    FLASH_ARGS=(
        flash
        --chip esp32c6
        --port "$PORT"
        --baud "$BAUD"
        --partition-table "$PARTITION_TABLE"
        --ignore-app-descriptor
    )

    if $DO_MONITOR; then
        FLASH_ARGS+=( --monitor )
    fi

    espflash "${FLASH_ARGS[@]}" "$ELF_PATH"

    # If --monitor was part of flash, we don't need to run monitor separately.
    if $DO_MONITOR; then
        exit 0
    fi

    echo "[flash] done!"
fi

# ── Monitor ───────────────────────────────────────────────────────────────

if $DO_MONITOR; then
    echo ""
    echo "[monitor] opening serial monitor on $PORT @ ${MONITOR_BAUD} baud..."
    echo "[monitor] press Ctrl+R to reset, Ctrl+C to exit"
    echo ""

    espflash monitor --port "$PORT" --baud "$MONITOR_BAUD"
fi
