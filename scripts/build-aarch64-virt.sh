#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="aarch64-unknown-none-softfloat"
PROFILE="${1:-debug}"
PKG="kernel-aarch64-virt"

if [[ "${PROFILE}" != "debug" && "${PROFILE}" != "release" ]]; then
    echo "usage: $0 [debug|release]" >&2
    exit 2
fi

cd "${ROOT}"

if command -v rustup >/dev/null 2>&1; then
    rustup target add "${TARGET}"
fi

if [[ "${PROFILE}" == "release" ]]; then
    cargo build --release -p "${PKG}" --target "${TARGET}"
    KERNEL="${ROOT}/target/${TARGET}/release/${PKG}"
else
    cargo build -p "${PKG}" --target "${TARGET}"
    KERNEL="${ROOT}/target/${TARGET}/debug/${PKG}"
fi

ALIAS_DIR="${ROOT}/build/veer-vm"
mkdir -p "${ALIAS_DIR}"
ALIAS="${ALIAS_DIR}/kernel-aarch64-${PROFILE}.elf"
cp "${KERNEL}" "${ALIAS}"

echo ""
echo "Built AArch64 virtual kernel: ${KERNEL}"
echo "Kernel alias: ${ALIAS}"
echo "Run with native Apple Silicon veer-vm once HVF-A64 run support is enabled:"
echo "  ./target/aarch64-apple-darwin/debug/veer-vm --arch aarch64 --kernel ${ALIAS} --memory 512 --hvf-run-once"
