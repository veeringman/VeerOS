#!/usr/bin/env bash
set -euo pipefail

# Validate folded ESP32-C6 + veer-connect on macOS across backend modes.
#
# Modes:
#  - qemu: rv32 guest via qemu fallback + hostfwd to localhost:2323
#  - soft: rv32-soft in-process backend; requires TAP networking on host
#
# Usage:
#   ./scripts/macos-esp32c6-connect-matrix.sh
#   ./scripts/macos-esp32c6-connect-matrix.sh --qemu-only
#   ./scripts/macos-esp32c6-connect-matrix.sh --soft-only
#   TAP=tap0 GUEST_IP=10.0.2.15 ./scripts/macos-esp32c6-connect-matrix.sh --soft-only

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE="${PROFILE:-debug}"
TAP="${TAP:-tap0}"
GUEST_IP="${GUEST_IP:-10.0.2.15}"
VSC_PORT="${VSC_PORT:-2323}"
TIMEOUT_SEC="${TIMEOUT_SEC:-25}"
CONNECT_TIMEOUT_SEC="${CONNECT_TIMEOUT_SEC:-20}"
MATRIX_USER="${MATRIX_USER:-user}"
MATRIX_PASS="${MATRIX_PASS:-veeros}"
RUN_QEMU=true
RUN_SOFT=true

say() { printf '\033[1;36m[connect-matrix]\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[connect-matrix]\033[0m %s\n' "$*"; }
die() { printf '\033[1;31m[connect-matrix]\033[0m %s\n' "$*" >&2; exit 1; }

require_cmd() { command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"; }

while [[ $# -gt 0 ]]; do
    case "$1" in
        --qemu-only)
            RUN_SOFT=false
            shift
            ;;
        --soft-only)
            RUN_QEMU=false
            shift
            ;;
        -h|--help)
            sed -n '2,38p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            die "unknown option: $1"
            ;;
    esac
done

[[ "$(uname -s)" == "Darwin" ]] || die "this script is macOS-only"

require_cmd cargo
require_cmd lsof
require_cmd python3

cd "$ROOT"

if [[ "$PROFILE" == "release" ]]; then
    VEER_CONNECT="${VEER_CONNECT:-$ROOT/target/release/veer-connect}"
    FOLD="${FOLD:-$ROOT/target/release/fold}"
else
    VEER_CONNECT="${VEER_CONNECT:-$ROOT/target/debug/veer-connect}"
    FOLD="${FOLD:-$ROOT/target/debug/fold}"
fi
KERNEL="${KERNEL:-$ROOT/target/riscv32imc-unknown-none-elf/$PROFILE/kernel-qemu-esp32c6}"

[[ -x "$VEER_CONNECT" ]] || die "veer-connect not built at $VEER_CONNECT"
[[ -x "$FOLD" ]] || die "fold not built at $FOLD"
[[ -f "$KERNEL" ]] || die "kernel not found at $KERNEL"

cleanup_name() {
    local name="$1"
    "$FOLD" stop "$name" >/dev/null 2>&1 || true
    "$FOLD" rm "$name" --force >/dev/null 2>&1 || true
}

wait_for_port_listener() {
    local port="$1"
    local timeout="$2"
    for _ in $(seq 1 "$timeout"); do
        if lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
            return 0
        fi
        sleep 1
    done
    return 1
}

wait_for_log_marker() {
    local name="$1"
    local timeout="$2"
    local pattern="$3"
    for _ in $(seq 1 "$timeout"); do
        if "$FOLD" logs "$name" 2>/dev/null | grep -Eq "$pattern"; then
            return 0
        fi
        sleep 1
    done
    return 1
}

run_connect_scripted() {
    local host="$1"
    local port="$2"
    local payload="$3"
    CONNECT_SCRIPT_PAYLOAD="$payload" python3 - "$VEER_CONNECT" "$host" "$port" "$CONNECT_TIMEOUT_SEC" <<'PY'
import os
import select
import subprocess
import sys
import time

binary, host, port, timeout_s = sys.argv[1:5]
payload = os.environ.get("CONNECT_SCRIPT_PAYLOAD", "")

proc = subprocess.Popen(
    [binary, "shell", host, port],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.STDOUT,
)

deadline = time.time() + float(timeout_s)
out = bytearray()
sent = False
disconnect_sent = False

expected_after_send = [
    b"scheduler ticks",
    b"ID  NAME",
    b"task table: temporarily unavailable",
]

while time.time() < deadline:
    if proc.stdout is None:
        break
    rlist, _, _ = select.select([proc.stdout], [], [], 0.2)
    if rlist:
        chunk = os.read(proc.stdout.fileno(), 4096)
        if not chunk:
            break
        out.extend(chunk)

    if (not sent) and (b"encrypted session established" in out or b"encrypted session (ChaCha20-Poly1305)" in out):
        if proc.stdin is not None:
            proc.stdin.write(payload.encode("utf-8"))
            proc.stdin.flush()
        sent = True

    if sent and (not disconnect_sent) and all(marker in out for marker in expected_after_send):
        if proc.stdin is not None:
            proc.stdin.write(b"\x1d\n")
            proc.stdin.flush()
        disconnect_sent = True

    if proc.poll() is not None and not rlist:
        break

if time.time() >= deadline and proc.poll() is None:
    proc.kill()
    out.extend(b"\n[connect-matrix] ERROR: scripted veer-connect session timed out\n")
    sys.stdout.buffer.write(out)
    sys.exit(124)

# Drain remaining output if process has exited.
if proc.stdout is not None:
    try:
        rest = proc.stdout.read()
    except Exception:
        rest = b""
    if rest:
        out.extend(rest)

sys.stdout.buffer.write(out)
sys.exit(proc.wait())
PY
}

assert_command_matrix() {
    local mode="$1"
    local out="$2"
    if grep -q 'scripted veer-connect session timed out' <<<"$out"; then
        die "[$mode] scripted session timed out"
    fi
    grep -q 'encrypted session' <<<"$out" || die "[$mode] missing encrypted session marker"
    grep -q 'scheduler ticks' <<<"$out" || die "[$mode] meminfo missing scheduler heartbeat"
    grep -q 'shell stack canary : ok' <<<"$out" || die "[$mode] shell stack canary not reported ok"
    grep -q 'remote stack canary:' <<<"$out" || die "[$mode] remote stack canary line missing"
    grep -q 'task table: temporarily unavailable' <<<"$out" || die "[$mode] ps fallback output missing"
    grep -q 'ID  NAME' <<<"$out" || die "[$mode] drivers table output missing"
}

run_qemu_test() {
    local name="esp32c6-qemu-connect-$$"
    cleanup_name "$name"
    trap 'cleanup_name "$name"' RETURN

    # Clear stale qemu listeners from prior interrupted runs.
    local pids
    pids="$(lsof -nP -t -iTCP:"$VSC_PORT" -sTCP:LISTEN 2>/dev/null || true)"
    if [[ -n "$pids" ]]; then
        warn "[qemu] clearing stale listeners on :$VSC_PORT ($pids)"
        kill $pids >/dev/null 2>&1 || true
        sleep 1
    fi

    say "[qemu] spawning folded guest with hostfwd on :$VSC_PORT"
    "$FOLD" vm spawn \
        --arch riscv32 \
        --riscv32-backend qemu \
        --vmnet shared \
        --kernel "$KERNEL" \
        --memory 128 \
        --name "$name" >/dev/null

    wait_for_log_marker "$name" "$TIMEOUT_SEC" 'DHCP complete|listening on port 2323' || {
        "$FOLD" logs "$name" | tail -n 80 || true
        die "[qemu] guest did not reach network-ready state"
    }

    wait_for_port_listener "$VSC_PORT" "$TIMEOUT_SEC" || {
        "$FOLD" logs "$name" | tail -n 80 || true
        die "[qemu] host listener on :$VSC_PORT did not appear"
    }

    say "[qemu] running scripted remote command matrix"
    local payload out
    payload=$(printf '%s\n%s\nhelp\nmeminfo\ndrivers\nps\nexit\n' "$MATRIX_USER" "$MATRIX_PASS")
    if ! out="$(run_connect_scripted 127.0.0.1 "$VSC_PORT" "$payload" 2>&1)"; then
        printf '%s\n' "$out"
        die "[qemu] scripted remote command matrix command failed"
    fi
    printf '%s\n' "$out"

    assert_command_matrix "qemu" "$out"

    say "[qemu] PASS: scripted remote command matrix completed"
}

run_soft_test() {
    local name="esp32c6-soft-connect-$$"
    cleanup_name "$name"
    trap 'cleanup_name "$name"' RETURN

    if [[ ! -c "/dev/$TAP" ]]; then
        warn "[soft] /dev/$TAP missing; TAP backend not provisioned on this host"
        warn "[soft] create/provision TAP first (usually requires tuntaposx + sudo ifconfig)"
        return 2
    fi

    say "[soft] spawning folded guest on TAP=$TAP"
    "$FOLD" vm spawn \
        --arch riscv32 \
        --riscv32-backend soft \
        --tap "$TAP" \
        --kernel "$KERNEL" \
        --memory 128 \
        --name "$name" >/dev/null

    wait_for_log_marker "$name" "$TIMEOUT_SEC" 'DHCP complete|listening on port 2323' || {
        "$FOLD" logs "$name" | tail -n 80 || true
        die "[soft] guest did not reach network-ready state"
    }

    say "[soft] running scripted remote command matrix via $GUEST_IP:$VSC_PORT"
    local payload out
    payload=$(printf '%s\n%s\nhelp\nmeminfo\ndrivers\nps\nexit\n' "$MATRIX_USER" "$MATRIX_PASS")
    if ! out="$(run_connect_scripted "$GUEST_IP" "$VSC_PORT" "$payload" 2>&1)"; then
        printf '%s\n' "$out"
        die "[soft] scripted remote command matrix command failed"
    fi
    printf '%s\n' "$out"

    assert_command_matrix "soft" "$out"

    say "[soft] PASS: scripted remote command matrix completed"
}

qemu_status=0
soft_status=0

if $RUN_QEMU; then
    run_qemu_test || qemu_status=$?
fi
if $RUN_SOFT; then
    run_soft_test || soft_status=$?
fi

if $RUN_QEMU && [[ $qemu_status -ne 0 ]]; then
    die "qemu backend connect validation failed"
fi
if $RUN_SOFT && [[ $soft_status -eq 1 ]]; then
    die "soft backend connect validation failed"
fi

if $RUN_SOFT && [[ $soft_status -eq 2 ]]; then
    warn "soft backend connect test blocked by missing TAP provisioning"
fi

say "validation complete"
