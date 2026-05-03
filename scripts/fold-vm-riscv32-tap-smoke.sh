#!/usr/bin/env bash
# Guarded smoke test for fold vm spawn with riscv32 + TAP forwarding.
#
# Flow: spawn -> wait for log pattern -> stop -> rm.
# The script is intentionally non-destructive and refuses to create TAP.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
FOLD="${FOLD:-$REPO/target/debug/fold}"
KERNEL="${KERNEL:-$REPO/target/riscv32imc-unknown-none-elf/debug/kernel-qemu-esp32c6}"
TAP="${TAP:-tap0}"
TIMEOUT_SEC="${TIMEOUT_SEC:-20}"
NAME="${NAME:-esp32c6-smoke-$(date +%s)}"
RV32_BACKEND="${RV32_BACKEND:-soft}"
HOST_OS="$(uname -s)"

say() { printf '\033[1;36m[fold-smoke]\033[0m %s\n' "$*"; }
die() { printf '\033[1;31m[fold-smoke]\033[0m %s\n' "$*" >&2; exit 1; }

cleanup() {
    "$FOLD" stop "$NAME" >/dev/null 2>&1 || true
    "$FOLD" rm "$NAME" --force >/dev/null 2>&1 || true
}
trap cleanup EXIT

[[ -x "$FOLD" ]] || die "fold not built at $FOLD (run: cargo build -p fold_engine)"
[[ -f "$KERNEL" ]] || die "kernel not found at $KERNEL (build riscv32 kernel first)"

SPAWN_ARGS=(
    vm spawn
    --arch riscv32
    --kernel "$KERNEL"
    --memory 128
    --riscv32-backend "$RV32_BACKEND"
    --user-ns
    --name "$NAME"
)

if [[ "$HOST_OS" == "Linux" ]]; then
    ip link show "$TAP" >/dev/null 2>&1 || die "required TAP interface missing: $TAP"
    SPAWN_ARGS+=(--tap "$TAP")
elif [[ "$HOST_OS" == "Darwin" ]]; then
    say "macOS detected: running folded rv32-soft smoke without TAP"
else
    die "unsupported host OS: $HOST_OS"
fi

say "spawning VM via fold (name=$NAME backend=$RV32_BACKEND)"
"$FOLD" "${SPAWN_ARGS[@]}" >/dev/null

say "waiting up to ${TIMEOUT_SEC}s for rv32 guest boot markers"
for _ in $(seq 1 "$TIMEOUT_SEC"); do
    logs="$($FOLD logs "$NAME" 2>/dev/null || true)"
    if printf '%s\n' "$logs" | grep -Eq 'rv32-soft: entering guest|\[boot\] starting scheduler'; then
        say "PASS: rv32 guest boot markers detected"
        printf '%s\n' "$logs" | grep -E 'rv32-soft: entering guest|\[boot\] starting scheduler' | head -n 3
        exit 0
    fi
    sleep 1
done

die "timeout waiting for rv32 boot markers in fold logs"
