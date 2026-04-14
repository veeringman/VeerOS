#!/usr/bin/env bash
#
# Build the VeerOS x86-64 kernel for QEMU q35/pc.
#
# Usage:
#   ./scripts/build-qemu-pc.sh            # debug build
#   ./scripts/build-qemu-pc.sh release    # release build
#
# Run with:
#   qemu-system-x86_64 -M q35 -m 256M -nographic -serial stdio \
#     -kernel target/x86_64-unknown-none/release/kernel-qemu-pc

set -euo pipefail

PROFILE="${1:-debug}"

if [ "$PROFILE" = "release" ]; then
    cargo build --release -p kernel-qemu-pc --target x86_64-unknown-none
    BINARY="target/x86_64-unknown-none/release/kernel-qemu-pc"
else
    cargo build -p kernel-qemu-pc --target x86_64-unknown-none
    BINARY="target/x86_64-unknown-none/debug/kernel-qemu-pc"
fi

echo ""
echo "Built: $BINARY"
echo ""
echo "Run with:"
echo "  qemu-system-x86_64 -M q35 -m 256M -nographic -serial stdio \\"
echo "    -kernel $BINARY"
