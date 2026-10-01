#!/usr/bin/env bash
# Run the AArch64 virt kernel under QEMU (HVF on Apple Silicon, TCG elsewhere)
# with a Bochs pixel display. Serial goes to a file; the monitor stays on
# stdio so callers can send `screendump <file>` / `quit`.
#
#   ./scripts/run-aarch64-virt-qemu.sh [debug|release]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROFILE="${1:-debug}"
KERNEL="${ROOT}/build/veer-vm/kernel-aarch64-${PROFILE}.elf"

if [[ ! -f "${KERNEL}" ]]; then
  echo "missing ${KERNEL} — run ./scripts/build-aarch64-virt.sh first" >&2
  exit 2
fi

ACCEL="hvf"
if [[ "$(uname -m)" != "arm64" ]]; then
  ACCEL="tcg"
fi

exec qemu-system-aarch64 \
  -M virt \
  -cpu host -accel "${ACCEL}" \
  -m 512 \
  -kernel "${KERNEL}" \
  -serial "file:${ROOT}/build/qemu-aarch64-serial.log" \
  -display none \
  -device bochs-display \
  -device virtio-keyboard-pci \
  -device virtio-tablet-pci \
  -device virtio-net-device,netdev=n0 -netdev user,id=n0 \
  -monitor stdio
