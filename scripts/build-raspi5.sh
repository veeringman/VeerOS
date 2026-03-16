#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────────
# VeerOS — RPi5 Build & Release Script
# ──────────────────────────────────────────────────────────────────────────
#
# Builds the kernel-raspi5 crate, converts ELF → raw binary (kernel8.img),
# and packages a ready-to-flash release archive.
#
# Usage:
#   ./scripts/build-raspi5.sh                  # default (dist-app, release)
#   ./scripts/build-raspi5.sh --dist minimal   # minimal distribution
#   ./scripts/build-raspi5.sh --dist rt        # real-time scheduler
#   ./scripts/build-raspi5.sh --dist full      # full (app + rt)
#   ./scripts/build-raspi5.sh --debug          # dev profile (unoptimised)
#   ./scripts/build-raspi5.sh --clean          # cargo clean first
#
# Output:
#   dist/raspi5/release/kernel8.img            — raw binary for SD card
#   dist/raspi5/release/kernel-raspi5.elf      — ELF (for debugging / GDB)
#   dist/raspi5/release/veeros-raspi5-v*.tar.gz — release archive
#
# Requirements:
#   - Rust toolchain with aarch64-unknown-none-softfloat target
#   - rust-objcopy  (from cargo-binutils)  OR  llvm-objcopy  OR  aarch64-none-elf-objcopy
#
set -euo pipefail

# ── Paths ─────────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
TARGET=aarch64-unknown-none-softfloat
PKG=kernel-raspi5
RELEASE_DIR="$ROOT_DIR/dist/raspi5/release"
BOOT_DIR="$ROOT_DIR/dist/raspi5/boot"

# ── Defaults ──────────────────────────────────────────────────────────────

DIST="app"
PROFILE="release"
DO_CLEAN=false

# ── Parse arguments ───────────────────────────────────────────────────────

while [[ $# -gt 0 ]]; do
    case "$1" in
        --dist)     DIST="$2";   shift 2 ;;
        --debug)    PROFILE="dev"; shift ;;
        --clean)    DO_CLEAN=true; shift ;;
        -h|--help)
            head -22 "$0" | tail -17
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            exit 1
            ;;
    esac
done

# ── Map distribution name → Cargo features ────────────────────────────────

case "$DIST" in
    minimal)  FEATURES="--no-default-features --features dist-minimal,shell" ;;
    app)      FEATURES="" ;;   # default features = dist-app
    rt)       FEATURES="--no-default-features --features dist-rt,shell,userlib,samples" ;;
    full)     FEATURES="--no-default-features --features dist-full" ;;
    *)
        echo "Error: unknown dist '$DIST'. Use: minimal | app | rt | full" >&2
        exit 1
        ;;
esac

# ── Resolve objcopy ──────────────────────────────────────────────────────

OBJCOPY=""
for candidate in rust-objcopy llvm-objcopy aarch64-none-elf-objcopy aarch64-linux-gnu-objcopy; do
    if command -v "$candidate" &>/dev/null; then
        OBJCOPY="$candidate"
        break
    fi
done

if [[ -z "$OBJCOPY" ]]; then
    echo "──────────────────────────────────────────────────────"
    echo "  ERROR: No objcopy found."
    echo ""
    echo "  Install one of:"
    echo "    cargo install cargo-binutils   # provides rust-objcopy"
    echo "    sudo apt install llvm          # provides llvm-objcopy"
    echo "    sudo apt install gcc-aarch64-none-elf"
    echo "──────────────────────────────────────────────────────"
    exit 1
fi

# ── Resolve version ──────────────────────────────────────────────────────

VERSION=$(grep '^version' "$ROOT_DIR/crates/kernel/raspi5/Cargo.toml" | head -1 | sed 's/.*"\(.*\)"/\1/')
GIT_SHORT=$(git -C "$ROOT_DIR" rev-parse --short HEAD 2>/dev/null || echo "unknown")

echo "╔══════════════════════════════════════════════════════╗"
echo "║  VeerOS RPi5 Build — v${VERSION} (${GIT_SHORT})             ║"
echo "╠══════════════════════════════════════════════════════╣"
echo "║  Distribution : dist-${DIST}"
echo "║  Profile      : ${PROFILE}"
echo "║  Target       : ${TARGET}"
echo "║  Objcopy      : ${OBJCOPY}"
echo "╚══════════════════════════════════════════════════════╝"
echo ""

# ── Optional clean ───────────────────────────────────────────────────────

if $DO_CLEAN; then
    echo "[1/5] Cleaning previous build..."
    cargo clean --manifest-path "$ROOT_DIR/Cargo.toml"
else
    echo "[1/5] Skipping clean (use --clean to force)"
fi

# ── Build ────────────────────────────────────────────────────────────────

echo "[2/5] Building kernel-raspi5 (dist-${DIST}, ${PROFILE})..."

BUILD_CMD="cargo build -p $PKG --target $TARGET"
if [[ "$PROFILE" == "release" ]]; then
    BUILD_CMD="$BUILD_CMD --release"
fi
# shellcheck disable=SC2086
eval "$BUILD_CMD $FEATURES"

# ── Locate ELF ───────────────────────────────────────────────────────────

if [[ "$PROFILE" == "release" ]]; then
    ELF_PATH="$ROOT_DIR/target/$TARGET/release/$PKG"
else
    ELF_PATH="$ROOT_DIR/target/$TARGET/debug/$PKG"
fi

if [[ ! -f "$ELF_PATH" ]]; then
    echo "Error: ELF not found at $ELF_PATH" >&2
    exit 1
fi

# ── ELF → raw binary ────────────────────────────────────────────────────

echo "[3/5] Converting ELF → kernel8.img (raw binary)..."
mkdir -p "$RELEASE_DIR"
"$OBJCOPY" -O binary "$ELF_PATH" "$RELEASE_DIR/kernel8.img"
cp "$ELF_PATH" "$RELEASE_DIR/kernel-raspi5.elf"

ELF_SIZE=$(stat -c%s "$ELF_PATH" 2>/dev/null || stat -f%z "$ELF_PATH")
IMG_SIZE=$(stat -c%s "$RELEASE_DIR/kernel8.img" 2>/dev/null || stat -f%z "$RELEASE_DIR/kernel8.img")
echo "       ELF size : $(numfmt --to=iec "$ELF_SIZE" 2>/dev/null || echo "${ELF_SIZE} bytes")"
echo "       IMG size : $(numfmt --to=iec "$IMG_SIZE" 2>/dev/null || echo "${IMG_SIZE} bytes")"

# ── Copy boot files ─────────────────────────────────────────────────────

echo "[4/5] Copying boot configuration files..."
cp "$BOOT_DIR/config.txt"  "$RELEASE_DIR/config.txt"
cp "$BOOT_DIR/cmdline.txt" "$RELEASE_DIR/cmdline.txt"

# ── Package release archive ─────────────────────────────────────────────

ARCHIVE_NAME="veeros-raspi5-v${VERSION}-${DIST}-${GIT_SHORT}.tar.gz"
echo "[5/5] Packaging release archive: ${ARCHIVE_NAME}"

tar -czf "$RELEASE_DIR/$ARCHIVE_NAME" \
    -C "$RELEASE_DIR" \
    kernel8.img \
    kernel-raspi5.elf \
    config.txt \
    cmdline.txt

echo ""
echo "════════════════════════════════════════════════════════"
echo "  BUILD COMPLETE"
echo ""
echo "  Release directory : dist/raspi5/release/"
echo "  kernel8.img       : $RELEASE_DIR/kernel8.img"
echo "  ELF (debug)       : $RELEASE_DIR/kernel-raspi5.elf"
echo "  Archive           : $RELEASE_DIR/$ARCHIVE_NAME"
echo ""
echo "  Next steps:"
echo "    ./scripts/deploy-sdcard.sh /dev/sdX   # Flash to SD card"
echo "    ./scripts/deploy-sdcard.sh --copy /mnt/boot  # Copy to mounted boot"
echo "════════════════════════════════════════════════════════"
