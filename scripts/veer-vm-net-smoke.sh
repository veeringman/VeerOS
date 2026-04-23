#!/usr/bin/env bash
# Smoke test for veer-vm virtio-net + TAP backend.
#
# Brings up tap0 on the host (10.0.2.2/24), boots kernel-qemu-pc under
# veer-vm, and pings the guest (10.0.2.15) from the host.
#
# Requires: sudo (once, to create tap), an x86_64 build of the kernel,
# and a release build of veer-vm.
set -euo pipefail

TAP="${TAP:-tap0}"
HOST_IP="${HOST_IP:-10.0.2.2}"
GUEST_IP="${GUEST_IP:-10.0.2.15}"
PREFIX="${PREFIX:-24}"

REPO="$(cd "$(dirname "$0")/.." && pwd)"
KERNEL="${KERNEL:-$REPO/target/x86_64-unknown-none/release/kernel-qemu-pc}"
VEER_VM="${VEER_VM:-$REPO/target/release/veer-vm}"

say() { printf '\033[1;36m[smoke]\033[0m %s\n' "$*"; }
die() { printf '\033[1;31m[smoke]\033[0m %s\n' "$*" >&2; exit 1; }

[[ -x "$VEER_VM" ]] || die "veer-vm not built: $VEER_VM (run: cargo build --release -p veer_vm)"
[[ -f "$KERNEL"  ]] || die "kernel not built: $KERNEL (run: ./scripts/build-qemu-pc.sh release)"

# --- TAP setup (idempotent) -------------------------------------------------
if ! ip link show "$TAP" >/dev/null 2>&1; then
    say "creating $TAP owned by $USER"
    sudo ip tuntap add dev "$TAP" mode tap user "$USER"
fi
if ! ip -4 addr show "$TAP" | grep -q "$HOST_IP/"; then
    say "assigning $HOST_IP/$PREFIX to $TAP"
    sudo ip addr add "$HOST_IP/$PREFIX" dev "$TAP" 2>/dev/null || true
fi
sudo ip link set "$TAP" up

# --- Launch veer-vm ---------------------------------------------------------
say "starting veer-vm (kernel=$KERNEL tap=$TAP)"
"$VEER_VM" --kernel "$KERNEL" --memory 128 --tap "$TAP" &
VM_PID=$!
trap 'kill -TERM "$VM_PID" 2>/dev/null || true; wait "$VM_PID" 2>/dev/null || true' EXIT

# Give the guest a moment to boot and bring up its interface.
say "waiting for guest $GUEST_IP to answer ARP/ICMP..."
for i in $(seq 1 30); do
    if ping -c 1 -W 1 "$GUEST_IP" >/dev/null 2>&1; then
        say "PING OK after ${i}s"
        ping -c 3 "$GUEST_IP" || true
        exit 0
    fi
    sleep 1
done

die "guest did not respond to ping within 30s (check veer-vm output above)"
