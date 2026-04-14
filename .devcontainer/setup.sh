#!/usr/bin/env bash
set -euo pipefail

echo "=== VeerOS devcontainer setup ==="

# System packages
sudo apt-get update -qq
sudo apt-get install -y -qq \
  qemu-system-x86 \
  qemu-system-arm \
  qemu-system-misc \
  grub-pc-bin \
  grub-common \
  xorriso \
  mtools \
  gdb-multiarch \
  novnc \
  websockify \
  nasm \
  > /dev/null

# Rust targets & components
rustup target add x86_64-unknown-none
rustup target add aarch64-unknown-none-softfloat
rustup target add riscv32imc-unknown-none-elf
rustup component add rust-src llvm-tools-preview

# Create build output directory
mkdir -p build

echo "=== Setup complete ==="
