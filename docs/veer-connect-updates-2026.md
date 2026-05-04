# veer-connect — Secure Connect Updates (2026)

## Overview
`veer-connect` provides secure, encrypted shell and file transfer for VeerOS nodes (including ESP32C6):

- **Modes**: `shell` (interactive terminal), `push` (upload), `pull` (download)
- **Security**: X25519 key exchange, VSC protocol, encrypted session
- **CLI**: Rust binary (`crates/veer-connect`), Python client (`scripts/veeros-connect`)
- **Integration**: Used by `veeros-vm` for ESP32C6 remote shell and file access
- **Progress feedback**: Shows upload/download progress and status

## 2026-05-04 Integration Notes (EdgeFabric + VeerOS Hosts)

- VeerOS discovered hosts should be routed to VeerOS shell transport (`veer-connect` style endpoint/port, commonly `2323`) when available.
- Do not assume all discovered hosts should default to SSH on `22`; this can produce false-positive "host reachable" results while shell attach fails.
- For dashboard/device UX, ensure host metadata includes explicit shell routing fields (host, port, agent id) so shell launch is deterministic.

### Quick verification

```bash
# Verify forwarded shell port is open on host
nc -zv <host> 2323

# Open VeerOS shell
veer-connect shell <host> 2323
```

If your deployment advertises a different shell port, use that discovered value directly.

## Example Usage
```
veer-connect shell <host> <port>
veer-connect push <host> <port> <local-path> <remote-path>
veer-connect pull <host> <port> <remote-path> <local-path>
```

## References
- [crates/veer-connect/](../crates/veer-connect/)
- [scripts/veeros-connect](../scripts/veeros-connect)
- [scripts/veeros-vm](../scripts/veeros-vm)

## Windows Test Notes: folded veer-vm + ESP32C6 guest

This section captures the current Windows test flow when validating `veer-connect` against an ESP32C6 guest launched via `veer-vm --backend custom`.

### Root cause of the failing run

**PowerShell ScriptBlock stripping** — In PowerShell, `{kernel}` without quotes is a ScriptBlock literal. When coerced to string it produces `"kernel"` (braces stripped). `veer-vm` receives the arg `kernel` instead of `{kernel}`, so the template substitution `a.replace("{kernel}", &kernel_path)` never matches, and QEMU is invoked as `-kernel kernel` (a nonexistent file). QEMU exits non-zero → veer-vm exits 1 → port 2323 never opens → `veer-connect` also fails.

`scripts/run-veer-vm-windows.ps1` currently only supports `-Arch x86_64`, so ESP32C6 (`riscv32`) testing must use direct `veer-vm.exe` invocation.

### Correct Windows launch pattern (ESP32C6)

The `{kernel}` template arg must be single-quoted so PowerShell passes the literal string:

```powershell
$kernel = '.\target\riscv32imc-unknown-none-elf\debug\kernel-qemu-esp32c6'
& .\target\x86_64-pc-windows-msvc\debug\veer-vm.exe `
  --backend custom --arch riscv32 `
  --kernel $kernel --memory 128 --cpus 1 `
  --custom-runner 'C:\Program Files\qemu\qemu-system-riscv32.exe' `
  --custom-arg=-M --custom-arg=virt `
  --custom-arg=-m --custom-arg=128M `
  --custom-arg=-nographic `
  --custom-arg=-bios --custom-arg=none `
  --custom-arg=-kernel --custom-arg='{kernel}' `
  --custom-arg=-netdev --custom-arg=user,id=n0,hostfwd=tcp::2323-:2323 `
  --custom-arg=-device --custom-arg=virtio-net-device,netdev=n0
```

Or avoid the template entirely and pass the kernel path directly (most robust):

```powershell
$kernel = (Resolve-Path .\target\riscv32imc-unknown-none-elf\debug\kernel-qemu-esp32c6).Path
& .\target\x86_64-pc-windows-msvc\debug\veer-vm.exe `
  --backend custom --arch riscv32 `
  --kernel $kernel --memory 128 --cpus 1 `
  --custom-runner 'C:\Program Files\qemu\qemu-system-riscv32.exe' `
  --custom-arg=-M --custom-arg=virt `
  --custom-arg=-m --custom-arg=128M `
  --custom-arg=-nographic `
  --custom-arg=-bios --custom-arg=none `
  --custom-arg=-kernel --custom-arg=$kernel `
  --custom-arg=-netdev --custom-arg=user,id=n0,hostfwd=tcp::2323-:2323 `
  --custom-arg=-device --custom-arg=virtio-net-device,netdev=n0
```

### Port conflict check (required)

Before starting the guest, verify the forwarded port is free:

```powershell
Get-NetTCPConnection -LocalPort 2323 -State Listen
```

If occupied, either stop that process or move to another host port (for example `4023`) and connect with:

```powershell
.\target\x86_64-pc-windows-msvc\debug\veer-connect.exe shell 127.0.0.1 4023
```

### Validation checklist

- Ensure `{kernel}` is single-quoted or the kernel path is expanded before launch.
- Launch `veer-vm` in a background terminal and keep the process running.
- Wait for QEMU boot output before connecting (or poll the port).
- Confirm a listener exists: `Get-NetTCPConnection -LocalPort 2323 -State Listen`
- Run `veer-connect shell <host> <port>` only after listener confirmation.
- If connect still fails, capture both `veer-vm` stderr and `Get-NetTCPConnection` output for the same run.

---

_Last updated: 2026-05-03_
