#!/usr/bin/env bash
set -euo pipefail

# Build VeerOS macOS host tools:
# - veer-vm (with HVF entitlement signing)
# - fold
# - veer-connect
#
# This is a convenience wrapper around scripts/build-mac.sh.
#
# Usage:
#   ./scripts/build-mac-host-tools.sh
#   ./scripts/build-mac-host-tools.sh release
#
# Environment overrides:
#   VEER_VM_MAC_TARGET=native|all|x86_64-apple-darwin|aarch64-apple-darwin
#   VEER_VM_MAC_PROFILE=debug|release
#
# Optional auto-sync from Linux host before building (macOS only):
#   VEEROS_PULL_FROM_LINUX=1        # default: disabled
#   LINUX_HOST=192.168.29.10        # required when pull is enabled
#   LINUX_USER=vijay                # default: $USER
#   LINUX_PORT=22                   # default: 22
#   LINUX_SRC=/home/vijay/rnd/VeerOS
#   LINUX_SSH_KEY=~/.ssh/id_ed25519

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE_ARG="${1:-}"
SCRIPT_PATH="${ROOT_DIR}/scripts/build-mac-host-tools.sh"
REEXEC_GUARD="${VEEROS_BUILD_MAC_HOST_TOOLS_REEXEC:-0}"

script_sha256() {
    local path="$1"
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$path" | awk '{print $1}'
    elif command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$path" | awk '{print $1}'
    else
        # No hash tool available; return empty and skip self-update detection.
        echo ""
    fi
}

HOST_OS="$(uname -s)"
VEEROS_PULL_FROM_LINUX="${VEEROS_PULL_FROM_LINUX:-}"

if [[ -z "${VEEROS_PULL_FROM_LINUX}" && "${HOST_OS}" == "Darwin" ]]; then
    VEEROS_PULL_FROM_LINUX=0
fi

if [[ "${VEEROS_PULL_FROM_LINUX:-0}" == "1" ]]; then
    LINUX_HOST="${LINUX_HOST:-}"
    LINUX_USER="${LINUX_USER:-${USER:-vijay}}"
    LINUX_PORT="${LINUX_PORT:-22}"
    LINUX_SRC="${LINUX_SRC:-/home/${LINUX_USER}/rnd/VeerOS}"
    LINUX_SSH_KEY="${LINUX_SSH_KEY:-}"

    if [[ -z "${LINUX_HOST}" ]]; then
        echo "ERROR: LINUX_HOST is required when VEEROS_PULL_FROM_LINUX=1"
        echo "Example: LINUX_HOST=192.168.29.10 ./scripts/build-mac-host-tools.sh"
        exit 1
    fi

    if ! command -v rsync >/dev/null 2>&1; then
        echo "ERROR: required tool not found: rsync"
        exit 1
    fi
    if ! command -v ssh >/dev/null 2>&1; then
        echo "ERROR: required tool not found: ssh"
        exit 1
    fi

    RSYNC_SSH="ssh -p ${LINUX_PORT}"
    if [[ -n "${LINUX_SSH_KEY}" ]]; then
        RSYNC_SSH+=" -i ${LINUX_SSH_KEY}"
    fi

    BEFORE_SCRIPT_SHA="$(script_sha256 "${SCRIPT_PATH}")"

    echo "▶ Pulling latest repo from Linux host ${LINUX_USER}@${LINUX_HOST}:${LINUX_SRC}/"
    rsync -az --delete \
        --exclude .git \
        --exclude target \
        --exclude .DS_Store \
        -e "${RSYNC_SSH}" \
        "${LINUX_USER}@${LINUX_HOST}:${LINUX_SRC}/" "${ROOT_DIR}/"

    AFTER_SCRIPT_SHA="$(script_sha256 "${SCRIPT_PATH}")"
    if [[ -n "${BEFORE_SCRIPT_SHA}" && -n "${AFTER_SCRIPT_SHA}" \
        && "${BEFORE_SCRIPT_SHA}" != "${AFTER_SCRIPT_SHA}" \
        && "${REEXEC_GUARD}" != "1" ]]; then
        echo "▶ build-mac-host-tools.sh was updated by sync; reloading updated script"
        exec env VEEROS_BUILD_MAC_HOST_TOOLS_REEXEC=1 "${SCRIPT_PATH}" "$@"
    fi
fi

if [[ "${PROFILE_ARG}" == "release" ]]; then
    export VEER_VM_MAC_PROFILE=release
elif [[ -n "${PROFILE_ARG}" && "${PROFILE_ARG}" != "debug" ]]; then
    echo "ERROR: unsupported profile argument: ${PROFILE_ARG}"
    echo "Usage: ./scripts/build-mac-host-tools.sh [debug|release]"
    exit 1
fi

cd "${ROOT_DIR}"
./scripts/build-mac.sh
