#!/usr/bin/env bash
set -euo pipefail

# Neutral alias for x86_64 ISO build path.
# Usage:
#   ./scripts/build-x86-iso.sh
#   ./scripts/build-x86-iso.sh release

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

exec ./scripts/build-qemu-pc.sh "${1:-debug}"
