#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────────
# VeerOS — RPi5 SD Card Deployment Script
# ──────────────────────────────────────────────────────────────────────────
#
# Deploys kernel8.img + config.txt to an SD card's boot partition.
#
# Usage:
#   # Copy to an already-mounted boot partition:
#   ./scripts/deploy-sdcard.sh --copy /media/vijay/bootfs
#
#   # Full SD card setup (format + copy firmware + VeerOS):
#   ./scripts/deploy-sdcard.sh --device /dev/sdb
#
#   # Just update kernel on a mounted partition (no firmware):
#   ./scripts/deploy-sdcard.sh --update /media/vijay/bootfs
#
# The --device mode will:
#   1. Partition the SD card (1 FAT32 boot partition)
#   2. Format as FAT32
#   3. Mount, copy RPi firmware files + VeerOS kernel
#   4. Unmount
#
# WARNING: --device mode ERASES the entire SD card!
#
# Requirements:
#   - Build first: ./scripts/build-raspi5.sh
#   - For --device: parted, mkfs.fat, sudo privileges
#   - RPi firmware files (start4.elf, fixup4.dat, bcm2712-*.dtb)
#     Download from: https://github.com/raspberrypi/firmware/tree/master/boot
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
RELEASE_DIR="$ROOT_DIR/dist/raspi5/release"
FIRMWARE_DIR="$ROOT_DIR/dist/raspi5/firmware"

# ── Colours ──────────────────────────────────────────────────────────────

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No colour

# ── Verify build artefacts exist ─────────────────────────────────────────

check_release() {
    if [[ ! -f "$RELEASE_DIR/kernel8.img" ]]; then
        echo -e "${RED}Error: kernel8.img not found in dist/raspi5/release/${NC}"
        echo "Run ./scripts/build-raspi5.sh first."
        exit 1
    fi
    local img_size
    img_size=$(stat -c%s "$RELEASE_DIR/kernel8.img" 2>/dev/null || stat -f%z "$RELEASE_DIR/kernel8.img")
    if [[ "$img_size" -lt 1024 ]]; then
        echo -e "${RED}Error: kernel8.img is suspiciously small (${img_size} bytes)${NC}"
        exit 1
    fi
    echo -e "${GREEN}Found kernel8.img ($(numfmt --to=iec "$img_size" 2>/dev/null || echo "${img_size}B"))${NC}"
}

# ── Mode: --copy (to mounted boot partition) ─────────────────────────────

do_copy() {
    local mount_point="$1"

    if [[ ! -d "$mount_point" ]]; then
        echo -e "${RED}Error: '$mount_point' is not a directory${NC}"
        exit 1
    fi

    echo "Deploying VeerOS to $mount_point ..."

    cp -v "$RELEASE_DIR/kernel8.img"  "$mount_point/kernel8.img"
    cp -v "$RELEASE_DIR/config.txt"   "$mount_point/config.txt"
    cp -v "$RELEASE_DIR/cmdline.txt"  "$mount_point/cmdline.txt"

    # Optionally copy ELF for on-device debugging reference
    if [[ -f "$RELEASE_DIR/kernel-raspi5.elf" ]]; then
        cp -v "$RELEASE_DIR/kernel-raspi5.elf" "$mount_point/kernel-raspi5.elf"
    fi

    sync
    echo ""
    echo -e "${GREEN}Deploy complete.${NC} Safe to eject the SD card."
    echo "Connect serial console (115200 baud, GPIO14/15) and power on."
}

# ── Mode: --update (kernel only, no config overwrite) ────────────────────

do_update() {
    local mount_point="$1"

    if [[ ! -d "$mount_point" ]]; then
        echo -e "${RED}Error: '$mount_point' is not a directory${NC}"
        exit 1
    fi

    echo "Updating kernel on $mount_point ..."
    cp -v "$RELEASE_DIR/kernel8.img" "$mount_point/kernel8.img"

    if [[ -f "$RELEASE_DIR/kernel-raspi5.elf" ]]; then
        cp -v "$RELEASE_DIR/kernel-raspi5.elf" "$mount_point/kernel-raspi5.elf"
    fi

    sync
    echo -e "${GREEN}Kernel updated.${NC} Safe to eject."
}

# ── Mode: --device (full SD card setup) ──────────────────────────────────

do_device() {
    local device="$1"

    # Safety: must be a block device, must NOT be a mounted root/boot device
    if [[ ! -b "$device" ]]; then
        echo -e "${RED}Error: '$device' is not a block device${NC}"
        exit 1
    fi

    # Refuse to operate on devices that look like the system disk
    local root_dev
    root_dev=$(findmnt -no SOURCE / 2>/dev/null | sed 's/[0-9]*$//' | sed 's/p[0-9]*$//' || true)
    if [[ -n "$root_dev" && "$device" == "$root_dev"* ]]; then
        echo -e "${RED}REFUSED: '$device' appears to be (part of) the root filesystem.${NC}"
        exit 1
    fi

    echo -e "${YELLOW}╔══════════════════════════════════════════════════╗${NC}"
    echo -e "${YELLOW}║  WARNING: This will ERASE ALL DATA on $device  ║${NC}"
    echo -e "${YELLOW}╚══════════════════════════════════════════════════╝${NC}"
    echo ""
    read -rp "Type YES to continue: " confirm
    if [[ "$confirm" != "YES" ]]; then
        echo "Aborted."
        exit 0
    fi

    echo ""
    echo "[1/5] Unmounting any existing partitions on $device ..."
    for part in "${device}"*; do
        if mountpoint -q "$part" 2>/dev/null || mount | grep -q "^$part "; then
            sudo umount "$part" 2>/dev/null || true
        fi
    done

    echo "[2/5] Creating partition table (MBR, single FAT32 partition) ..."
    sudo parted -s "$device" mklabel msdos
    sudo parted -s "$device" mkpart primary fat32 1MiB 256MiB
    sudo parted -s "$device" set 1 boot on
    sleep 1  # wait for kernel to re-read partition table

    # Detect partition name (sdb1 vs sdb-part1 vs mmcblk0p1)
    local part1=""
    for candidate in "${device}1" "${device}p1" "${device}-part1"; do
        if [[ -b "$candidate" ]]; then
            part1="$candidate"
            break
        fi
    done
    if [[ -z "$part1" ]]; then
        echo -e "${RED}Error: Could not find partition 1 on $device${NC}"
        exit 1
    fi

    echo "[3/5] Formatting $part1 as FAT32 (label: VEEROS) ..."
    sudo mkfs.fat -F 32 -n VEEROS "$part1"

    echo "[4/5] Mounting and copying files ..."
    local tmp_mount
    tmp_mount=$(mktemp -d)
    sudo mount "$part1" "$tmp_mount"

    # Copy RPi firmware if available
    if [[ -d "$FIRMWARE_DIR" ]]; then
        echo "       Copying RPi firmware files ..."
        for fw_file in "$FIRMWARE_DIR"/*; do
            if [[ -f "$fw_file" ]]; then
                sudo cp -v "$fw_file" "$tmp_mount/"
            fi
        done
    else
        echo -e "${YELLOW}       WARNING: No firmware dir at dist/raspi5/firmware/${NC}"
        echo "       You'll need to manually copy RPi firmware files:"
        echo "         start4.elf, fixup4.dat, bcm2712-rpi-5-b.dtb"
        echo "       Download from: https://github.com/raspberrypi/firmware/tree/master/boot"
    fi

    # Copy VeerOS files
    echo "       Copying VeerOS kernel + config ..."
    sudo cp -v "$RELEASE_DIR/kernel8.img"  "$tmp_mount/kernel8.img"
    sudo cp -v "$RELEASE_DIR/config.txt"   "$tmp_mount/config.txt"
    sudo cp -v "$RELEASE_DIR/cmdline.txt"  "$tmp_mount/cmdline.txt"

    if [[ -f "$RELEASE_DIR/kernel-raspi5.elf" ]]; then
        sudo cp -v "$RELEASE_DIR/kernel-raspi5.elf" "$tmp_mount/kernel-raspi5.elf"
    fi

    echo "[5/5] Syncing and unmounting ..."
    sync
    sudo umount "$tmp_mount"
    rmdir "$tmp_mount"

    echo ""
    echo -e "${GREEN}════════════════════════════════════════════════════${NC}"
    echo -e "${GREEN}  SD CARD READY${NC}"
    echo ""
    echo "  Device     : $device"
    echo "  Partition  : $part1 (FAT32, label=VEEROS)"
    echo ""
    echo "  Insert into RPi5, connect serial (115200, GPIO14/15),"
    echo "  and power on to boot VeerOS."
    echo -e "${GREEN}════════════════════════════════════════════════════${NC}"
}

# ── Main ─────────────────────────────────────────────────────────────────

usage() {
    echo "Usage:"
    echo "  $0 --copy   <mount_point>   Copy VeerOS to mounted boot partition"
    echo "  $0 --update <mount_point>   Update kernel only (no config overwrite)"
    echo "  $0 --device <block_device>  Full SD card setup (ERASES card!)"
    echo ""
    echo "Examples:"
    echo "  $0 --copy /media/vijay/bootfs"
    echo "  $0 --update /media/vijay/bootfs"
    echo "  $0 --device /dev/sdb"
}

if [[ $# -lt 2 ]]; then
    usage
    exit 1
fi

check_release

case "$1" in
    --copy)    do_copy "$2"   ;;
    --update)  do_update "$2" ;;
    --device)  do_device "$2" ;;
    *)
        usage
        exit 1
        ;;
esac
