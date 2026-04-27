# macOS HVF Quick Start Guide

## Prerequisites

- **macOS**: Monterey 12.0+ (Intel or Apple Silicon)
- **Hardware**: Intel Mac with VMX-capable CPU or Apple Silicon Mac
- **Command-line tools**: Xcode Command Line Tools installed
  ```bash
  xcode-select --install
  ```

## One-Command Build & Sign

```bash
./scripts/build-mac.sh
```

Or use the explicit host-tools wrapper:

```bash
./scripts/build-mac-host-tools.sh
```

## Sync To macOS Host

From your Linux/dev machine, sync and verify the latest sources on the Mac:

```bash
MAC_HOST=192.168.29.74 MAC_USER=vijay ./scripts/sync-mac.sh
```

The script syncs the repository (excluding `.git` and `target`) and verifies
SHA-256 checksums for key HVF/signing files.

Note: If you are using password auth (no SSH key), you may be prompted during
both sync and verification steps.

Optional overrides:
```bash
# Build release instead of debug
VEER_VM_MAC_PROFILE=release ./scripts/build-mac.sh

# Use explicit target (for future variants)
VEER_VM_MAC_TARGET=x86_64-apple-darwin ./scripts/build-mac.sh

# Sync with an explicit SSH key
MAC_HOST=192.168.29.74 MAC_USER=vijay MAC_SSH_KEY=~/.ssh/id_ed25519 ./scripts/sync-mac.sh
```

This automatically:
1. Compiles veer-vm for macOS
2. Compiles Fold (`fold`) and `veer-connect` for macOS
3. Embeds hypervisor entitlements in veer-vm
4. Code-signs the veer-vm binary
5. Verifies the veer-vm signature

## Verify Signing Succeeded

```bash
codesign --verify --verbose=4 target/x86_64-apple-darwin/debug/veer-vm
codesign --display --entitlements - target/x86_64-apple-darwin/debug/veer-vm
```

Expected output:
```
com.apple.security.hypervisor
<true/>
```

## Run a Kernel

```bash
# Build kernel artifact alias for veer-vm/fold workflows
./scripts/build-x86-vm.sh

./target/x86_64-apple-darwin/debug/veer-vm \
  --kernel build/veer-vm/kernel-x86_64-debug.elf \
  --memory 256
```

## Probe HVF Readiness (No Kernel Required)

```bash
# Preferred explicit form:
./target/x86_64-apple-darwin/debug/veer-vm --hvf-probe

# Compatibility alias:
./target/x86_64-apple-darwin/debug/veer-vm --probe
```

## Troubleshooting

### Error: `-85377017 (HV_BUSY)`

**Problem**: veer-vm binary is not properly signed with hypervisor entitlement.

**Solution**:
```bash
rm -f target/x86_64-apple-darwin/debug/veer-vm
./scripts/build-mac.sh

# If CARGO_TARGET_DIR is set in your shell, verify the printed binary path
# from build-mac.sh and inspect that exact file with codesign.
```

### Error: `kern.hv_support != 1`

**Problem**: Hypervisor.framework not available on this machine.

**Check**:
```bash
sysctl kern.hv_support
```

**Requirements**:
- Native macOS (not inside a VM)
- Monterey 12.0+
- Intel Mac with VMX OR Apple Silicon

### Error: `codesign: Permission denied`

**Problem**: User doesn't have code-signing permissions.

**Solution**:
```bash
# May need sudo, but this is unusual for ad-hoc signing:
sudo codesign --force --sign - --entitlements crates/veer_vm/veer-vm.entitlements target/x86_64-apple-darwin/debug/veer-vm
```

## System Capability Check

```bash
echo "=== Hypervisor Support ==="
sysctl kern.hv_support kern.hv_vmm_present
echo ""
echo "=== CPU Features ==="
sysctl -n machdep.cpu.features | grep -o "VMX\|AVX\|AES" | sort
echo ""
echo "=== macOS Version ==="
sw_vers
```

## Advanced: Release Build with Developer ID

For distribution or publishing:

```bash
# Get your Developer ID
security find-identity -v -p codesigning

# Build and sign with Developer ID (replace with your identity hash)
cargo build -p veer_vm --target x86_64-apple-darwin --release
codesign --force \
  --sign "Developer ID Application: Your Name (XXXXXXXXXX)" \
  --entitlements crates/veer_vm/veer-vm.entitlements \
  target/x86_64-apple-darwin/release/veer-vm

# Notarize (if needed for public release)
xcrun notarytool submit target/x86_64-apple-darwin/release/veer-vm \
  --apple-id your-apple-id@icloud.com \
  --password your-app-specific-password \
  --wait
```

## Debugging with LLDB

```bash
lldb -- ./target/x86_64-apple-darwin/debug/veer-vm --kernel <path> --memory 256
(lldb) br set -n main
(lldb) br set -n probe  # Break at HVF probe
(lldb) run
(lldb) continue
```

## Next Steps

- Check [docs/macos-hypervisor.md](./macos-hypervisor.md) for full architecture details
- See [docs/veer-vm-updates-2026.md](./veer-vm-updates-2026.md) for feature roadmap
- Explore [docs/architecture.md](./architecture.md) for VeerOS design

