#!/usr/bin/env bash
#
# Run VeerOS on QEMU q35 (x86-64).
#
# Networking modes (set NET_MODE):
#   nat     — (default) QEMU user-net with NAT + port forwarding.
#             Guest gets 10.0.2.15 via DHCP, host forwards SSH port.
#             No root required. Guest can reach the internet.
#   bridge  — TAP device bridged to host LAN. Guest gets a real LAN IP
#             via DHCP from your router. Requires root or CAP_NET_ADMIN,
#             and a pre-configured bridge (e.g. br0).
#
# Environment variables:
#   NET_MODE        nat | bridge              (default: nat)
#   BRIDGE          bridge interface name     (default: br0)
#   TAP_IFACE       tap device name           (default: tap-veeros)
#   HOST_SSH_PORT   host port for SSH fwd     (default: 2222, nat only)
#   HOST_BIND_ADDR  bind address for fwd      (default: 0.0.0.0, nat only)
#   GUEST_SSH_PORT  guest SSH port            (default: 2222, nat only)
#   MEMORY_MB       guest RAM in MiB          (default: 256)
#   DISPLAY_MODE    serial | vga              (default: serial)
#   DISK_IMG        path to secondary disk    (default: none)
#   DISK_SIZE       create disk if missing    (default: 64M)

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROFILE="${1:-debug}"
ISO="$ROOT/build/veeros.iso"

NET_MODE="${NET_MODE:-nat}"
BRIDGE="${BRIDGE:-br0}"
TAP_IFACE="${TAP_IFACE:-tap-veeros}"
HOST_SSH_PORT="${HOST_SSH_PORT:-2222}"
HOST_BIND_ADDR="${HOST_BIND_ADDR:-0.0.0.0}"
GUEST_SSH_PORT="${GUEST_SSH_PORT:-2222}"
MEMORY_MB="${MEMORY_MB:-256}"
DISPLAY_MODE="${DISPLAY_MODE:-serial}"
DISK_IMG="${DISK_IMG:-}"
DISK_SIZE="${DISK_SIZE:-64M}"

if ! command -v qemu-system-x86_64 >/dev/null 2>&1; then
    echo "qemu-system-x86_64 not found" >&2
    exit 1
fi

if [ ! -f "$ISO" ]; then
    "$ROOT/scripts/build-qemu-pc.sh" "$PROFILE"
fi

qemu_args=(
    -M q35
    -m "$MEMORY_MB"
    -cdrom "$ISO"
)

# ── Networking ────────────────────────────────────────────────────────
case "$NET_MODE" in
    nat)
        qemu_args+=(
            -netdev "user,id=net0,hostfwd=tcp:${HOST_BIND_ADDR}:${HOST_SSH_PORT}-:${GUEST_SSH_PORT}"
            -device virtio-net-pci,netdev=net0
        )
        NET_INFO="NAT (user-net) — SSH: ${HOST_BIND_ADDR}:${HOST_SSH_PORT} -> guest:${GUEST_SSH_PORT}"
        ;;
    bridge)
        # Create TAP device and attach to bridge.
        # Requires: bridge (br0) already configured on host.
        # Setup:
        #   sudo ip link add br0 type bridge
        #   sudo ip link set br0 up
        #   sudo ip link set <host-eth> master br0
        #   sudo dhclient br0   (or configure static IP on br0)
        if [ "$(id -u)" -ne 0 ]; then
            echo "Bridge mode requires root. Run with: sudo NET_MODE=bridge $0" >&2
            exit 1
        fi
        if ! ip link show "$BRIDGE" &>/dev/null; then
            echo "Bridge '$BRIDGE' not found. Create it first:" >&2
            echo "  sudo ip link add $BRIDGE type bridge" >&2
            echo "  sudo ip link set $BRIDGE up" >&2
            echo "  sudo ip link set <your-eth-iface> master $BRIDGE" >&2
            exit 1
        fi
        # Create and configure TAP.
        ip tuntap add dev "$TAP_IFACE" mode tap 2>/dev/null || true
        ip link set "$TAP_IFACE" up
        ip link set "$TAP_IFACE" master "$BRIDGE" 2>/dev/null || true

        qemu_args+=(
            -netdev "tap,id=net0,ifname=${TAP_IFACE},script=no,downscript=no"
            -device virtio-net-pci,netdev=net0
        )
        NET_INFO="Bridge ($BRIDGE via $TAP_IFACE) — guest gets LAN IP via DHCP"

        # Clean up TAP on exit.
        cleanup() {
            ip link set "$TAP_IFACE" nomaster 2>/dev/null || true
            ip link delete "$TAP_IFACE" 2>/dev/null || true
        }
        trap cleanup EXIT
        ;;
    *)
        echo "Unsupported NET_MODE: $NET_MODE" >&2
        echo "Use NET_MODE=nat or NET_MODE=bridge" >&2
        exit 1
        ;;
esac

# ── Secondary disk (virtio-blk) ──────────────────────────────────────
DISK_INFO=""
if [ -n "$DISK_IMG" ]; then
    if [ ! -f "$DISK_IMG" ]; then
        echo "Creating FAT32 disk image: $DISK_IMG ($DISK_SIZE)"
        qemu-img create -f raw "$DISK_IMG" "$DISK_SIZE"
        # Format as FAT32 with MBR partition table.
        # Create a single partition spanning the whole disk.
        /sbin/mkfs.vfat -F 32 "$DISK_IMG"
    fi
    qemu_args+=(
        -drive "file=${DISK_IMG},format=raw,if=none,id=disk0"
        -device virtio-blk-pci,drive=disk0
        -boot d
    )
    DISK_INFO=" | disk: $DISK_IMG"
fi

# ── KVM acceleration ─────────────────────────────────────────────────
if [ -r /dev/kvm ]; then
    qemu_args+=(-accel kvm)
    ACCEL="kvm"
else
    ACCEL="tcg"
fi

# ── Display ───────────────────────────────────────────────────────────
case "$DISPLAY_MODE" in
    serial)
        qemu_args+=(-display none -serial stdio)
        ;;
    vga)
        ;;
    *)
        echo "Unsupported DISPLAY_MODE: $DISPLAY_MODE" >&2
        echo "Use DISPLAY_MODE=serial or DISPLAY_MODE=vga" >&2
        exit 1
        ;;
esac

echo "Starting QEMU ($ACCEL) — $NET_INFO$DISK_INFO"
if [ "$NET_MODE" = "nat" ]; then
    echo "Connect: ssh -p ${HOST_SSH_PORT} veeros@<host-ip>"
else
    echo "Connect: ssh veeros@<guest-dhcp-ip>  (check router or boot log for IP)"
fi

exec qemu-system-x86_64 "${qemu_args[@]}"