#!/usr/bin/env bash
# Smoke test for veer-vm snapshot/restore foundation.
#
# Validates the no-virtio path by:
#   1) Booting a VeerOS image with --snapshot-save.
#   2) Logging in non-interactively and issuing a marker command.
#   3) Shutting down via timeout->SIGTERM (clean path, snapshot written).
#   4) Restoring via --restore and checking shell history for the marker.
#
# If TAP is set (e.g. TAP=tap0), also validates virtio-net snapshot files and
# restores with the same TAP attached.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
VEER_VM="${VEER_VM:-$REPO/target/debug/veer-vm}"
IMAGE="${IMAGE:-$REPO/build/veeros.iso}"
MEMORY_MIB="${MEMORY_MIB:-128}"
SAVE_TIMEOUT_SECS="${SAVE_TIMEOUT_SECS:-14}"
RESTORE_TIMEOUT_SECS="${RESTORE_TIMEOUT_SECS:-8}"
TAP="${TAP:-}"

SNAP_DIR="${SNAP_DIR:-/tmp/veeros-snap-smoke-$$}"
LOG_SAVE="${LOG_SAVE:-/tmp/veeros-snap-save-$$.log}"
LOG_RESTORE="${LOG_RESTORE:-/tmp/veeros-snap-restore-$$.log}"
INPUT_SAVE="/tmp/veeros-snap-save-input-$$.txt"
INPUT_RESTORE="/tmp/veeros-snap-restore-input-$$.txt"

say() { printf '\033[1;36m[snap-smoke]\033[0m %s\n' "$*"; }
die() { printf '\033[1;31m[snap-smoke]\033[0m %s\n' "$*" >&2; exit 1; }

cleanup() {
    rm -f "$INPUT_SAVE" "$INPUT_RESTORE"
}
trap cleanup EXIT

[[ -x "$VEER_VM" ]] || die "veer-vm not found: $VEER_VM (run: cargo build -p veer_vm)"
[[ -f "$IMAGE" ]] || die "boot image not found: $IMAGE"

cat > "$INPUT_SAVE" <<'EOF'
root
toor
echo before-snapshot
EOF

cat > "$INPUT_RESTORE" <<'EOF'
history
EOF

rm -rf "$SNAP_DIR"

save_args=(--kernel "$IMAGE" --memory "$MEMORY_MIB" --snapshot-save "$SNAP_DIR")
restore_args=(--restore "$SNAP_DIR")
if [[ -n "$TAP" ]]; then
    save_args+=(--tap "$TAP")
    restore_args+=(--tap "$TAP")
fi

say "save run: booting with --snapshot-save $SNAP_DIR"
set +e
timeout --signal=TERM --kill-after=3s "$SAVE_TIMEOUT_SECS" \
    "$VEER_VM" "${save_args[@]}" \
    < "$INPUT_SAVE" > "$LOG_SAVE" 2>&1
save_rc=$?
set -e
if [[ "$save_rc" -ne 0 && "$save_rc" -ne 124 ]]; then
    tail -n 80 "$LOG_SAVE" >&2 || true
    die "save run failed (rc=$save_rc)"
fi

for file in meta.bin memory.bin regs.bin sregs.bin lapic.bin mp_state.bin vcpu_events.bin xsave.bin xcrs.bin debugregs.bin pic_master.bin pic_slave.bin ioapic.bin pit.bin serial.bin; do
    [[ -f "$SNAP_DIR/$file" ]] || die "missing snapshot file: $SNAP_DIR/$file"
done

if [[ -n "$TAP" ]]; then
    [[ -f "$SNAP_DIR/net.present" ]] || die "missing snapshot file: $SNAP_DIR/net.present"
    [[ -f "$SNAP_DIR/net.bin" ]] || die "missing snapshot file: $SNAP_DIR/net.bin"
    [[ -f "$SNAP_DIR/net.tap" ]] || die "missing snapshot file: $SNAP_DIR/net.tap"
fi

grep -q "snapshot saved to $SNAP_DIR" "$LOG_SAVE" || die "save log missing snapshot completion line"

say "restore run: restoring from $SNAP_DIR"
set +e
timeout --signal=TERM --kill-after=3s "$RESTORE_TIMEOUT_SECS" \
    "$VEER_VM" "${restore_args[@]}" \
    < "$INPUT_RESTORE" > "$LOG_RESTORE" 2>&1
restore_rc=$?
set -e
if [[ "$restore_rc" -ne 0 && "$restore_rc" -ne 124 ]]; then
    tail -n 80 "$LOG_RESTORE" >&2 || true
    die "restore run failed (rc=$restore_rc)"
fi

grep -q "restored snapshot $SNAP_DIR" "$LOG_RESTORE" || die "restore log missing restore line"
grep -q "echo before-snapshot" "$LOG_RESTORE" || die "restore log missing expected shell history marker"

say "PASS"
say "save log:    $LOG_SAVE"
say "restore log: $LOG_RESTORE"
say "snapshot:    $SNAP_DIR"
