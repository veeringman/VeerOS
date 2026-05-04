#!/usr/bin/env bash
# Smoke test for veer-vm virtio-net + TAP backend.
#
# Brings up tap0 on the host (10.0.2.2/24), boots kernel-qemu-pc under
# veer-vm, and pings the guest (10.0.2.15) from the host.
#
# Requires: sudo (once, to create tap), an x86_64 build of the kernel,
# and a release build of veer-vm.
set -euo pipefail

HOST_IP="${HOST_IP:-10.0.2.2}"
GUEST_IP="${GUEST_IP:-10.0.2.15}"
PREFIX="${PREFIX:-24}"
TAP="${TAP:-tap0}"
VMNET_MODE="${VMNET_MODE:-shared}"

REPO="$(cd "$(dirname "$0")/.." && pwd)"
HOST_OS="$(uname -s)"
HOST_ARCH="$(uname -m)"

# Default to AArch64 guest boot on Apple Silicon hosts.
VM_ARCH="${VM_ARCH:-}"
if [[ -z "${VM_ARCH}" && "${HOST_OS}" == "Darwin" && "${HOST_ARCH}" == "arm64" ]]; then
    VM_ARCH="aarch64"
fi

if [[ "$HOST_OS" == "Darwin" ]]; then
    case "${VEER_VM_MAC_TARGET:-native}" in
        native|all)
            case "$(uname -m)" in
                arm64|aarch64) TARGET_TRIPLE="aarch64-apple-darwin" ;;
                x86_64|amd64) TARGET_TRIPLE="x86_64-apple-darwin" ;;
                *) die "unsupported macOS host architecture: $(uname -m)" ;;
            esac
            ;;
        *) TARGET_TRIPLE="${VEER_VM_MAC_TARGET}" ;;
    esac
    PROFILE="${VEER_VM_MAC_PROFILE:-debug}"
    VEER_VM_DEFAULT="$REPO/target/$TARGET_TRIPLE/$PROFILE/veer-vm"
else
    VEER_VM_DEFAULT="$REPO/target/release/veer-vm"
fi
VEER_VM="${VEER_VM:-$VEER_VM_DEFAULT}"

# Pick the first existing kernel artifact, preferring a profile that matches
# the host-tool profile on macOS.
if [[ "$HOST_OS" == "Darwin" ]]; then
    KERNEL_PROFILE_PRIMARY="${VEER_VM_MAC_PROFILE:-debug}"
else
    KERNEL_PROFILE_PRIMARY="release"
fi

if [[ "$KERNEL_PROFILE_PRIMARY" == "release" ]]; then
    KERNEL_PROFILE_SECONDARY="debug"
else
    KERNEL_PROFILE_SECONDARY="release"
fi

DEFAULT_KERNEL=""
if [[ "${VM_ARCH}" == "aarch64" ]]; then
    for candidate in \
        "$REPO/build/veer-vm/kernel-aarch64-$KERNEL_PROFILE_PRIMARY.elf" \
        "$REPO/build/veer-vm/kernel-aarch64-$KERNEL_PROFILE_SECONDARY.elf" \
        "$REPO/target/aarch64-unknown-none-softfloat/$KERNEL_PROFILE_PRIMARY/kernel-aarch64-virt" \
        "$REPO/target/aarch64-unknown-none-softfloat/$KERNEL_PROFILE_SECONDARY/kernel-aarch64-virt"
    do
        if [[ -f "$candidate" ]]; then
            DEFAULT_KERNEL="$candidate"
            break
        fi
    done
else
    for candidate in \
        "$REPO/target/x86_64-unknown-none/$KERNEL_PROFILE_PRIMARY/kernel-x86_64-pc" \
        "$REPO/target/x86_64-unknown-none/$KERNEL_PROFILE_PRIMARY/kernel-qemu-pc" \
        "$REPO/target/x86_64-unknown-none/$KERNEL_PROFILE_SECONDARY/kernel-x86_64-pc" \
        "$REPO/target/x86_64-unknown-none/$KERNEL_PROFILE_SECONDARY/kernel-qemu-pc" \
        "$REPO/build/veer-vm/kernel-x86_64-$KERNEL_PROFILE_PRIMARY.elf" \
        "$REPO/build/veer-vm/kernel-x86_64-$KERNEL_PROFILE_SECONDARY.elf"
    do
        if [[ -f "$candidate" ]]; then
            DEFAULT_KERNEL="$candidate"
            break
        fi
    done
fi

if [[ -z "$DEFAULT_KERNEL" ]]; then
    if [[ "${VM_ARCH}" == "aarch64" ]]; then
        DEFAULT_KERNEL="$REPO/build/veer-vm/kernel-aarch64-$KERNEL_PROFILE_PRIMARY.elf"
    else
        DEFAULT_KERNEL="$REPO/target/x86_64-unknown-none/$KERNEL_PROFILE_PRIMARY/kernel-x86_64-pc"
    fi
fi

KERNEL="${KERNEL:-$DEFAULT_KERNEL}"

say() { printf '\033[1;36m[smoke]\033[0m %s\n' "$*"; }
die() { printf '\033[1;31m[smoke]\033[0m %s\n' "$*" >&2; exit 1; }
require_cmd() { command -v "$1" >/dev/null 2>&1 || die "missing command '$1' (install required host tooling)"; }

ping_once() {
    if [[ "$HOST_OS" == "Linux" ]]; then
        ping -c 1 -W 1 "$GUEST_IP" >/dev/null 2>&1
    else
        # macOS ping uses milliseconds for -W.
        ping -c 1 -W 1000 "$GUEST_IP" >/dev/null 2>&1
    fi
}

require_cmd ping
require_cmd sudo

[[ -x "$VEER_VM" ]] || die "veer-vm not built: $VEER_VM (run: cargo build --release -p veer_vm)"
[[ -f "$KERNEL"  ]] || die "kernel not built: $KERNEL (run: ./scripts/build-veer-vm.sh ${KERNEL_PROFILE_PRIMARY})"

# --- TAP setup (idempotent) -------------------------------------------------
if [[ "$HOST_OS" == "Linux" ]]; then
    require_cmd ip
    if ! ip link show "$TAP" >/dev/null 2>&1; then
        say "creating $TAP owned by $USER"
        sudo ip tuntap add dev "$TAP" mode tap user "$USER"
    fi
    if ! ip -4 addr show "$TAP" | grep -q "$HOST_IP/"; then
        say "assigning $HOST_IP/$PREFIX to $TAP"
        sudo ip addr add "$HOST_IP/$PREFIX" dev "$TAP" 2>/dev/null || true
    fi
    sudo ip link set "$TAP" up
elif [[ "$HOST_OS" == "Darwin" ]]; then
    case "$VMNET_MODE" in
        shared|host) ;;
        *) die "invalid VMNET_MODE=$VMNET_MODE (expected: shared or host)" ;;
    esac
    say "macOS vmnet mode: $VMNET_MODE"
else
    die "unsupported host OS: $HOST_OS"
fi

# --- Launch veer-vm ---------------------------------------------------------
if [[ "$HOST_OS" == "Darwin" ]]; then
    say "starting veer-vm (kernel=$KERNEL arch=${VM_ARCH:-auto} vmnet=$VMNET_MODE)"
    if [[ -n "${VM_ARCH}" ]]; then
        "$VEER_VM" --arch "$VM_ARCH" --kernel "$KERNEL" --memory 128 --vmnet "$VMNET_MODE" &
    else
        "$VEER_VM" --kernel "$KERNEL" --memory 128 --vmnet "$VMNET_MODE" &
    fi
else
    say "starting veer-vm (kernel=$KERNEL tap=$TAP)"
    "$VEER_VM" --kernel "$KERNEL" --memory 128 --tap "$TAP" &
fi
VM_PID=$!
trap 'kill -TERM "$VM_PID" 2>/dev/null || true; wait "$VM_PID" 2>/dev/null || true' EXIT

# Give the guest a moment to boot and bring up its interface.
say "waiting for guest $GUEST_IP to answer ARP/ICMP..."
for i in $(seq 1 30); do
    if [[ "$HOST_OS" == "Darwin" ]]; then
        if ! kill -0 "$VM_PID" 2>/dev/null; then
            die "veer-vm exited early in vmnet mode"
        fi
        if [[ "$i" -ge 8 ]]; then
            say "vmnet mode launch stable after ${i}s (network ping check pending vmnet backend wiring)"
            exit 0
        fi
    elif ping_once; then
        say "PING OK after ${i}s"
        ping -c 3 "$GUEST_IP" || true
        exit 0
    fi
    sleep 1
done

die "guest did not respond to ping within 30s (check veer-vm output above)"
