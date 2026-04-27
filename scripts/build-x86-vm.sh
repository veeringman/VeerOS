#!/usr/bin/env bash
set -euo pipefail

# Neutral alias for veer-vm/fold x86_64 kernel + VMM build.
# Usage:
#   ./scripts/build-x86-vm.sh
#   ./scripts/build-x86-vm.sh release

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

exec ./scripts/build-veer-vm.sh "${1:-debug}"
