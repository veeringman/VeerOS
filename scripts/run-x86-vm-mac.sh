#!/usr/bin/env bash
set -euo pipefail

# Neutral alias for running veer-vm on macOS.
# Usage:
#   ./scripts/run-x86-vm-mac.sh [kernel-elf] [extra veer-vm args...]

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

exec ./scripts/run-mac.sh "$@"
