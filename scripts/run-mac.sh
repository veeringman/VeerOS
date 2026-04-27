#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_TRIPLE="${VEER_VM_MAC_TARGET:-x86_64-apple-darwin}"
PROFILE="${VEER_VM_MAC_PROFILE:-debug}"

if [[ "${PROFILE}" == "release" ]]; then
  VEER_VM_BIN="${ROOT_DIR}/target/${TARGET_TRIPLE}/release/veer-vm"
else
  VEER_VM_BIN="${ROOT_DIR}/target/${TARGET_TRIPLE}/debug/veer-vm"
fi

kernel="${1:-}"
if [[ -n "$kernel" ]]; then
  shift
else
  if [[ -f "build/veer-vm/kernel-x86_64-debug.elf" ]]; then
    kernel="build/veer-vm/kernel-x86_64-debug.elf"
  elif [[ -f "build/veer-vm/kernel-x86_64-release.elf" ]]; then
    kernel="build/veer-vm/kernel-x86_64-release.elf"
  else
    echo "usage: $0 [kernel-elf] [extra veer-vm args...]" >&2
    echo "hint: run ./scripts/build-veer-vm.sh first" >&2
    exit 2
  fi
fi

if [[ ! -f "${VEER_VM_BIN}" ]]; then
  echo "veer-vm binary not found: ${VEER_VM_BIN}" >&2
  echo "hint: run ./scripts/build-mac-host-tools.sh first" >&2
  exit 1
fi

"${VEER_VM_BIN}" \
  --kernel "$kernel" \
  --memory 128 \
  "$@"
