#!/usr/bin/env bash
set -euo pipefail

# Sync VeerOS workspace to a macOS host and verify key files match.
#
# Usage:
#   MAC_HOST=192.168.29.74 MAC_USER=vijay ./scripts/sync-mac.sh
# Optional:
#   MAC_PORT=22
#   MAC_DEST=/Users/vijay/rnd/VeerOS
#
# Notes:
# - This script uses rsync over ssh.
# - Authentication is delegated to your ssh setup (keys/agent/password prompt).

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAC_HOST="${MAC_HOST:-}"
MAC_USER="${MAC_USER:-vijay}"
MAC_PORT="${MAC_PORT:-22}"
MAC_DEST="${MAC_DEST:-/Users/${MAC_USER}/rnd/VeerOS}"
MAC_SSH_KEY="${MAC_SSH_KEY:-}"

if [[ -z "${MAC_HOST}" ]]; then
    echo "ERROR: MAC_HOST is required"
    echo "Example: MAC_HOST=192.168.29.74 MAC_USER=vijay ./scripts/sync-mac.sh"
    exit 1
fi

if [[ -n "${MAC_SSH_KEY}" && ! -f "${MAC_SSH_KEY}" ]]; then
    echo "ERROR: MAC_SSH_KEY does not exist: ${MAC_SSH_KEY}"
    exit 1
fi

require_tool() {
    local tool="$1"
    if ! command -v "${tool}" >/dev/null 2>&1; then
        echo "ERROR: required tool not found: ${tool}"
        exit 1
    fi
}

require_tool rsync
require_tool ssh
require_tool sha256sum

SSH_TARGET="${MAC_USER}@${MAC_HOST}"
SSH_OPTS=( -p "${MAC_PORT}" )
RSYNC_SSH="ssh -p ${MAC_PORT}"

if [[ -n "${MAC_SSH_KEY}" ]]; then
    SSH_OPTS+=( -i "${MAC_SSH_KEY}" )
    RSYNC_SSH+=" -i ${MAC_SSH_KEY}"
fi

echo "▶ Syncing ${ROOT_DIR}/ -> ${SSH_TARGET}:${MAC_DEST}/"
rsync -az --delete \
    --exclude .git \
    --exclude target \
    --exclude .DS_Store \
    -e "${RSYNC_SSH}" \
    "${ROOT_DIR}/" "${SSH_TARGET}:${MAC_DEST}/"

echo "▶ Verifying key files by SHA-256"
KEY_FILES=(
    "scripts/build-mac.sh"
    "scripts/build-mac-host-tools.sh"
    "scripts/sync-mac.sh"
    "crates/veer_vm/src/main.rs"
    "crates/veer_vm/src/backend/hvf.rs"
    "crates/veer_vm/build.rs"
    "crates/veer_vm/veer-vm.entitlements"
    "crates/veer-connect/Cargo.toml"
    "crates/veer-connect/src/main.rs"
    "crates/veer-connect/src/terminal.rs"
    "crates/veer-connect/src/transfer.rs"
    "docs/QUICKSTART-macOS-HVF.md"
)

for rel in "${KEY_FILES[@]}"; do
    local_path="${ROOT_DIR}/${rel}"
    if [[ ! -f "${local_path}" ]]; then
        echo "ERROR: missing local file ${rel}"
        exit 1
    fi
done

remote_manifest="$(
    ssh "${SSH_OPTS[@]}" "${SSH_TARGET}" sh -s -- "${MAC_DEST}" "${KEY_FILES[@]}" <<'REMOTE_EOF'
set -eu

dest="$1"
shift
cd "$dest"

for rel in "$@"; do
    if [ -f "$rel" ]; then
        sum="$(shasum -a 256 "$rel" | sed 's/[[:space:]].*$//')"
        printf '%s\t%s\n' "$sum" "$rel"
    else
        printf 'MISSING\t%s\n' "$rel"
    fi
done
REMOTE_EOF
)"

for rel in "${KEY_FILES[@]}"; do
    local_path="${ROOT_DIR}/${rel}"
    local_sha="$(sha256sum "${local_path}" | awk '{print $1}')"
    remote_sha="$(grep -F $'\t'"${rel}" <<<"${remote_manifest}" | head -n1 | cut -f1)"

    if [[ -z "${remote_sha}" ]]; then
        echo "ERROR: remote checksum output missing for ${rel}"
        exit 1
    fi

    if [[ "${remote_sha}" != "${local_sha}" ]]; then
        echo "ERROR: SHA mismatch for ${rel}"
        echo "  local : ${local_sha}"
        echo "  remote: ${remote_sha}"
        exit 1
    fi

done

echo "✓ Sync complete and verified"
echo "  Remote path: ${MAC_DEST}"
