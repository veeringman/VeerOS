# veer-vm — Recent Updates (2026)

## Overview
`veer-vm` is a lightweight KVM-based microVMM for VeerOS, designed as a Firecracker-class alternative to QEMU.

### Key Features
- KVM orchestration: boots VeerOS x86_64/esp32c6 kernels
- Virtio-mmio transport: console, block, net
- Direct ISO boot (no ELF required)
- Snapshot/restore (RAM, vCPU, devices)
- Sensor feed: inject sensor data via FIFO for EdgeFabric/IoT
- Integration with Fold: `fold vm spawn` wraps veer-vm in a secure envelope
- Rootless-friendly defaults

### Usage
See [crates/veer_vm/README.md](../crates/veer_vm/README.md) for CLI and architecture details.

### Roadmap
- aarch64 KVM backend (RPi5 guest)
- Live migration
- More virtio devices

---

_Last updated: 2026-04-25_
