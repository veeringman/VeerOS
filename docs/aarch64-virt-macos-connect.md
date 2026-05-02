# AArch64 macOS Veer-VM Connect Bring-Up

This tracks the Apple Silicon path for running a VeerOS AArch64 guest under `veer-vm` and reaching it from macOS with `veer-connect` and SSH.

## Current Verified Path

Build the generic AArch64 guest kernel:

```bash
./scripts/build-aarch64-virt.sh debug
```

Build native macOS host tools:

```bash
./scripts/build-mac-host-tools.sh
```

Boot the guest in bounded HVF mode:

```bash
VEER_VM_HVF_RUN_ONCE=1 \
  ./target/aarch64-apple-darwin/debug/veer-vm \
  --arch aarch64 \
  --kernel build/veer-vm/kernel-aarch64-debug.elf \
  --memory 512 \
  --hvf-run-once
```

Expected serial banner:

```text
VeerOS AArch64 virtual target v0.1.0
platform : AArch64 virtual machine
uart     : PL011 @ 0x09000000
load     : 0x00080000
status   : booted; parking until scheduler bring-up
```

## Connect Smoke

```bash
./scripts/aarch64-virt-connect-smoke.sh
```

Today this exits `78` after checking the current bring-up state. The Apple Silicon HVF backend now has a vmnet-backed virtio-mmio host device window, but the AArch64 guest still needs the matching virtio-mmio driver and SSH service wiring before `veer-connect` can pass.

If macOS returns `VMNET_INVALID_ACCESS` or `VMNET_NOT_AUTHORIZED`, the current non-admin user may not be able to approve the vmnet operation. Use the admin user `vijaysharma` if macOS prompts for approval or policy changes.

## Required Work For veer-connect/SSH

1. Continue hardening the host-side virtio-mmio net device in `crates/veer_vm/src/backend/hvf_aarch64.rs`.
   - Current MMIO window: `0x0a00_0000`.
   - RX/TX are backed by macOS `vmnet` in `shared` or `host` mode.
   - Next host-side work is interrupt delivery/GIC integration and broader virtqueue-chain handling.

2. Add a guest-side virtio-mmio net driver under `crates/soc/aarch64_virt`.
   - Mirror the existing `soc-qemu-pc` virtio-net semantics, but use MMIO transport instead of legacy PCI I/O ports.
   - Expose an `arch::NetworkDevice` adapter for the AArch64 kernel.

3. Lift the existing TCP/SSH service stack from `kernel-qemu-pc` into `kernel-aarch64-virt`.
   - Add `net`, `smoltcp`, `ssh`, `crypto`, `shell`, and `userlib` dependencies/features.
   - Start the network poll task.
   - Start the SSH listener on port `22`.

4. Make the smoke script pass by running:

```bash
./target/aarch64-apple-darwin/debug/veer-vm \
  --arch aarch64 \
  --kernel build/veer-vm/kernel-aarch64-debug.elf \
  --memory 512 \
  --vmnet shared

./target/aarch64-apple-darwin/debug/veer-connect shell 10.0.2.15 22
```
