#!/usr/bin/env bash
set -euo pipefail

# Build macOS host artifacts used for HVF bring-up.
cargo build -p veer_vm --target x86_64-apple-darwin
cargo build -p fold_engine --target x86_64-apple-darwin
