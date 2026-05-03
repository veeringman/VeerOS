#!/usr/bin/env bash
set -euo pipefail

# One-command macOS bootstrap for folded ESP32-C6 runs.
#
# What it does:
#  1) Builds mac host tools (veer-vm, fold, veer-connect) via scripts/build-mac.sh
#  2) Builds ESP32-C6 kernel image via scripts/build-qemu-esp32c6.sh
#  3) Runs folded smoke for rv32-soft backend
#  4) Optionally runs folded smoke for qemu backend (if qemu-system-riscv32 exists)
#
# Usage:
#   ./scripts/macos-esp32c6-fold-bootstrap.sh
#   ./scripts/macos-esp32c6-fold-bootstrap.sh --release
#   ./scripts/macos-esp32c6-fold-bootstrap.sh --skip-build
#   ./scripts/macos-esp32c6-fold-bootstrap.sh --install-qemu
#   ./scripts/macos-esp32c6-fold-bootstrap.sh --soft-only
#   ./scripts/macos-esp32c6-fold-bootstrap.sh --connect-matrix

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE="debug"
SKIP_BUILD=false
INSTALL_QEMU=false
SOFT_ONLY=false
CONNECT_MATRIX=false
TIMEOUT_SEC="${TIMEOUT_SEC:-20}"

say() { printf '\033[1;36m[mac-esp32c6]\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[mac-esp32c6]\033[0m %s\n' "$*"; }
die() { printf '\033[1;31m[mac-esp32c6]\033[0m %s\n' "$*" >&2; exit 1; }

require_cmd() {
    command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --release)
            PROFILE="release"
            shift
            ;;
        --skip-build)
            SKIP_BUILD=true
            shift
            ;;
        --install-qemu)
            INSTALL_QEMU=true
            shift
            ;;
        --soft-only)
            SOFT_ONLY=true
            shift
            ;;
        --connect-matrix)
            CONNECT_MATRIX=true
            shift
            ;;
        -h|--help)
            sed -n '2,40p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            die "unknown option: $1"
            ;;
    esac
done

[[ "$(uname -s)" == "Darwin" ]] || die "this script is macOS-only"

require_cmd cargo
require_cmd bash

cd "$ROOT"

if ! $SKIP_BUILD; then
    say "building mac host tools (profile=$PROFILE)"
    VEER_VM_MAC_PROFILE="$PROFILE" ./scripts/build-mac.sh

    say "building ESP32-C6 kernel image (profile=$PROFILE)"
    if [[ "$PROFILE" == "release" ]]; then
        ./scripts/build-qemu-esp32c6.sh release
    else
        ./scripts/build-qemu-esp32c6.sh
    fi
else
    say "skipping build phase (--skip-build)"
fi

KERNEL="$ROOT/target/riscv32imc-unknown-none-elf/$PROFILE/kernel-qemu-esp32c6"
[[ -f "$KERNEL" ]] || die "kernel not found: $KERNEL"

say "running folded smoke: rv32-soft"
TIMEOUT_SEC="$TIMEOUT_SEC" KERNEL="$KERNEL" RV32_BACKEND=soft ./scripts/fold-vm-riscv32-tap-smoke.sh

if $SOFT_ONLY; then
    say "done (soft-only mode)"
    exit 0
fi

if ! command -v qemu-system-riscv32 >/dev/null 2>&1; then
    if $INSTALL_QEMU; then
        if command -v brew >/dev/null 2>&1; then
            say "qemu-system-riscv32 missing; installing qemu with Homebrew"
            brew install qemu
        else
            warn "qemu-system-riscv32 missing and Homebrew not found; skipping qemu backend smoke"
            say "done (soft backend validated; qemu backend skipped)"
            exit 0
        fi
    else
        warn "qemu-system-riscv32 missing; skipping qemu backend smoke"
        warn "install with: brew install qemu  (or rerun with --install-qemu)"
        say "done (soft backend validated; qemu backend skipped)"
        exit 0
    fi
fi

say "running folded smoke: qemu fallback"
TIMEOUT_SEC="$TIMEOUT_SEC" KERNEL="$KERNEL" RV32_BACKEND=qemu ./scripts/fold-vm-riscv32-tap-smoke.sh

if $CONNECT_MATRIX; then
    say "running veer-connect backend matrix"
    TIMEOUT_SEC="$TIMEOUT_SEC" PROFILE="$PROFILE" ./scripts/macos-esp32c6-connect-matrix.sh
fi

say "done (soft + qemu backend paths validated)"
