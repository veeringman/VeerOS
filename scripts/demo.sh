#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
# VeerOS AI-Native Demo Launcher
# ─────────────────────────────────────────────────────────────────────
#
# Run the VeerOS host-demo binary interactively.
# The shell supports full AI-native commands:
#
#   agents   — spawn / list / kill autonomous agents
#   intent   — submit / decompose / track high-level goals
#   memory   — persistent key-value store (get / set / delete)
#   fabric   — view execution fabric node topology
#   demo     — run scripted walkthrough scenarios
#
# Quick start:
#   ./scripts/demo.sh                    # interactive shell
#   ./scripts/demo.sh --scenario deploy  # auto-run deploy demo
#
# ─────────────────────────────────────────────────────────────────────

set -euo pipefail
cd "$(dirname "$0")/.."

echo "Building VeerOS demo..."
cargo build -p veeros-demo --quiet 2>/dev/null || cargo build -p veeros-demo

DEMO_BIN="./target/debug/veeros-demo"

if [[ "${1:-}" == "--scenario" && -n "${2:-}" ]]; then
    # Non-interactive: pipe a scenario + exit
    printf 'demo %s\nexit\n' "$2" | "$DEMO_BIN"
else
    # Interactive mode
    exec "$DEMO_BIN"
fi
