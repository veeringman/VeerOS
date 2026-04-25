#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "usage: $0 <kernel-elf> [extra veer-vm args...]" >&2
  exit 2
fi

kernel="$1"
shift

cargo run -p veer_vm --target x86_64-apple-darwin -- \
  --kernel "$kernel" \
  --memory 128 \
  "$@"
