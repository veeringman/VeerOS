#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROFILE="${VEER_VM_MAC_PROFILE:-debug}"
VMNET_MODE="${VMNET_MODE:-shared}"
GUEST_IP="${GUEST_IP:-10.0.2.15}"
SSH_PORT="${SSH_PORT:-2323}"

case "${PROFILE}" in
    debug|release) ;;
    *) echo "usage: VEER_VM_MAC_PROFILE=debug|release $0" >&2; exit 2 ;;
esac

if [[ "$(uname -s)" != "Darwin" || "$(uname -m)" != "arm64" ]]; then
    echo "ERROR: this smoke test targets Apple Silicon macOS" >&2
    exit 2
fi

KERNEL="${KERNEL:-${ROOT}/build/veer-vm/kernel-aarch64-${PROFILE}.elf}"
VEER_VM="${VEER_VM:-${ROOT}/target/aarch64-apple-darwin/${PROFILE}/veer-vm}"
VEER_CONNECT="${VEER_CONNECT:-${ROOT}/target/aarch64-apple-darwin/${PROFILE}/veer-connect}"

say() { printf '\033[1;36m[aarch64-connect]\033[0m %s\n' "$*"; }
die() { printf '\033[1;31m[aarch64-connect]\033[0m %s\n' "$*" >&2; exit 1; }

[[ -f "${KERNEL}" ]] || die "AArch64 kernel not built: ${KERNEL} (run: ./scripts/build-aarch64-virt.sh ${PROFILE})"
[[ -x "${VEER_VM}" ]] || die "native veer-vm not built: ${VEER_VM} (run: ./scripts/build-mac-host-tools.sh ${PROFILE})"
[[ -x "${VEER_CONNECT}" ]] || die "native veer-connect not built: ${VEER_CONNECT} (run: ./scripts/build-mac-host-tools.sh ${PROFILE})"

say "kernel      : ${KERNEL}"
say "veer-vm     : ${VEER_VM}"
say "veer-connect: ${VEER_CONNECT}"
say "vmnet       : ${VMNET_MODE}"
say "guest ssh   : ${GUEST_IP}:${SSH_PORT}"

set +e
OUTPUT=$("${VEER_VM}" --arch aarch64 --kernel "${KERNEL}" --memory 512 --vmnet "${VMNET_MODE}" --hvf-run-once 2>&1)
STATUS=$?
set -e

printf '%s\n' "${OUTPUT}"

if [[ ${STATUS} -ne 0 ]] && grep -q "vmnet_start_interface" <<<"${OUTPUT}"; then
    say "blocked by macOS vmnet authorization/policy; admin user vijaysharma may be needed if macOS requests approval"
    exit 78
fi

if [[ ${STATUS} -ne 0 ]]; then
    exit "${STATUS}"
fi

say "guest started with vmnet-backed virtio-mmio host device"

# Give the guest a moment to bring up networking, then connect.
say "waiting for guest VSC to come up..."
sleep 3

say "connecting via VSC on ${GUEST_IP}:${SSH_PORT}"
"${VEER_CONNECT}" shell "${GUEST_IP}" "${SSH_PORT}"
