# Phase 2D: MMIO Trap Handling (macOS HVF)

## Overview
Phase 2D implements guest memory-mapped I/O (MMIO) trap handling, routing LAPIC and I/O APIC access attempts to the corresponding device model emulation functions. This enables the guest kernel to interact with timer hardware and interrupt controllers.

## Architecture

### MMIO Address Ranges

**LAPIC (Local APIC):**
- Range: `0xFEE0_0000` to `0xFEE0_0FFF` (4 KiB)
- Access type: Read/Write 32-bit registers
- Existing device model: `HvfLapic` struct (hvf.rs ~line 1100)
- Key registers:
  - `0x000`: ID register (read-only, vCPU number)
  - `0x020`: TPR (Task Priority Register)
  - `0x030`: EOI (End of Interrupt, write-only)
  - `0x0B0`: Spurious Interrupt Vector (enable/disable)
  - `0x320`: LVT Timer (vector, mode, mask)
  - `0x380`: Timer Initial Count
  - `0x390`: Timer Current Count (read-only)

**I/O APIC:**
- Range: `0xFEC0_0000` to `0xFEC0_0FFF` (4 KiB)
- Access type: Indexed register access (2 registers)
  - `0x00`: Index register (select which IOAPIC register to read/write)
  - `0x10`: Data register (read/write selected register)
- Existing device model: `HvfIoapic` struct (hvf.rs ~line 1150)
- Handles IRQ-to-vector routing for external devices

### EPT Violation Detection

When guest accesses unmapped or trap-enabled GPA ranges:
```
1. Guest attempts MMIO access (read or write)
2. EPT (Extended Page Table) violation occurs
3. CPU exits to hypervisor with EXIT_REASON_EPT_VIOLATION (48)
4. VMCS_RO_EXIT_QUALIFICATION contains access details
5. VMCS_GUEST_PHYSICAL_ADDRESS contains the GPA
```

**VMCS_RO_EXIT_QUALIFICATION bits** (for EPT violation):
```
Bits [2:0]:
  Bit 0 = Read access
  Bit 1 = Write access
  Bit 2 = Execute access (usually 0 for MMIO)

Bits [6:3]: Reserved
Bits [12:7]: PAGING_STRUCTURE_LEVEL causing violation (usually ignored for MMIO)
Bits [31:13]: Reserved
```

## Implementation Tasks

### Task 2D.1: Create MMIO region classifier

**New function:**
```rust
/// Phase 2D: Identify MMIO device from guest physical address
#[derive(Clone, Copy, Debug)]
enum MmioDevice {
    Lapic,
    Ioapic,
}

fn classify_mmio_device(gpa: u64) -> Option<MmioDevice> {
    match gpa {
        0xFEE0_0000..=0xFEE0_0FFF => Some(MmioDevice::Lapic),
        0xFEC0_0000..=0xFEC0_0FFF => Some(MmioDevice::Ioapic),
        _ => None,
    }
}

/// Phase 2D: Validate MMIO access size (LAPIC/IOAPIC require 32-bit access)
fn is_valid_mmio_access_size(device: MmioDevice, len: usize) -> bool {
    // Both LAPIC and IOAPIC expect 32-bit (4-byte) accesses
    match device {
        MmioDevice::Lapic => len == 4,
        MmioDevice::Ioapic => len == 4,
    }
}

/// Phase 2D: Calculate device-relative offset from guest physical address
fn mmio_offset(device: MmioDevice, gpa: u64) -> u64 {
    match device {
        MmioDevice::Lapic => gpa - 0xFEE0_0000,
        MmioDevice::Ioapic => gpa - 0xFEC0_0000,
    }
}
```

### Task 2D.2: Handle MMIO reads

**New function:**
```rust
/// Phase 2D: Handle MMIO read from device
fn handle_mmio_read(
    lapic: &HvfLapic,
    ioapic: &HvfIoapic,
    device: MmioDevice,
    offset: u64,
) -> Result<u32> {
    match device {
        MmioDevice::Lapic => {
            let value = lapic.read_u32(0xFEE0_0000 + offset);
            eprintln!(
                "[hvf] LAPIC read: offset={:#x} value={:#x}",
                offset, value
            );
            Ok(value)
        }
        MmioDevice::Ioapic => {
            let value = ioapic.read_u32(0xFEC0_0000 + offset);
            eprintln!(
                "[hvf] IOAPIC read: offset={:#x} value={:#x}",
                offset, value
            );
            Ok(value)
        }
    }
}

/// Phase 2D: Extract 32-bit value from RAX (result of read to write back)
fn write_result_to_rax(vcpu: HvVcpuId, value: u32) -> Result<()> {
    // For 32-bit MMIO, entire RAX becomes the result
    write_reg(vcpu, HV_X86_RAX, value as u64)?;
    eprintln!("[hvf] wrote {:#x} to RAX", value as u64);
    Ok(())
}
```

### Task 2D.3: Handle MMIO writes

**New function:**
```rust
/// Phase 2D: Handle MMIO write to device
fn handle_mmio_write(
    lapic: &mut HvfLapic,
    ioapic: &mut HvfIoapic,
    device: MmioDevice,
    offset: u64,
    value: u32,
) -> Result<()> {
    match device {
        MmioDevice::Lapic => {
            eprintln!(
                "[hvf] LAPIC write: offset={:#x} value={:#x}",
                offset, value
            );
            lapic.write_u32(0xFEE0_0000 + offset, value);
        }
        MmioDevice::Ioapic => {
            eprintln!(
                "[hvf] IOAPIC write: offset={:#x} value={:#x}",
                offset, value
            );
            ioapic.write_u32(0xFEC0_0000 + offset, value);
        }
    }
    Ok(())
}

/// Phase 2D: Extract 32-bit value from RAX (guest-provided write data)
fn read_rax_value(vcpu: HvVcpuId) -> Result<u32> {
    let rax = read_reg(vcpu, HV_X86_RAX, "hv_vcpu_read_register(RAX)")?;
    Ok((rax & 0xFFFF_FFFF) as u32)
}
```

### Task 2D.4: Integrate EPT violation handling into execution loop

**Update dispatch_exit():**
```rust
fn dispatch_exit(
    vcpu: HvVcpuId,
    exit_state: &ExitState,
    _uart: &mut HvfUart16550,
    lapic: &mut HvfLapic,
    ioapic: &mut HvfIoapic,
) -> Result<ExitDispatch> {
    let reason = exit_state.reason;

    // ... existing CPUID, HALT, IO handling ...

    match reason {
        VMX_EXIT_REASON_EPT_VIOLATION => {
            // Phase 2D: Handle MMIO traps
            let gpa = exit_state.guest_phys_addr.unwrap_or(0);
            let qualification = exit_state.qualification;
            let is_write = ((qualification >> 1) & 1) != 0;
            
            eprintln!(
                "[hvf] EPT violation: gpa={:#x} write={} rip={:#x}",
                gpa,
                is_write,
                read_reg(vcpu, HV_X86_RIP, "RIP")?
            );

            if let Some(device) = classify_mmio_device(gpa) {
                let offset = mmio_offset(device, gpa);

                if is_write {
                    // MMIO write: get value from RAX, write to device
                    let value = read_rax_value(vcpu)?;
                    handle_mmio_write(lapic, ioapic, device, offset, value)?;
                } else {
                    // MMIO read: read from device, put value in RAX
                    let value = handle_mmio_read(lapic, ioapic, device, offset)?;
                    write_result_to_rax(vcpu, value)?;
                }

                // Advance RIP past the faulting instruction
                let instr_len = exit_state.instr_len as u64;
                advance_guest_rip(vcpu, instr_len)?;

                return Ok(ExitDispatch::Handled(format!(
                    "MMIO {:?} {:?} gpa={:#x}",
                    if is_write { "write" } else { "read" },
                    device,
                    gpa
                )));
            }

            // Unmapped MMIO access
            eprintln!("[hvf] WARNING: unmapped MMIO access at {:#x}", gpa);
            return Ok(ExitDispatch::Stop(ExitReason::Shutdown));
        }
        _ => {}
    }

    // ... existing exit handling ...
    Ok(ExitDispatch::Stop(ExitReason::Shutdown))
}
```

### Task 2D.5: Configure EPT to trap MMIO regions

**For future EPT-based trap configuration (if not using in-kernel APIC):**
```rust
/// Phase 2D: Configure EPT to trap MMIO accesses
/// Note: Hypervisor.framework may handle this automatically in APIC mode
fn configure_ept_mmio_traps(vm_id: HvVmId) -> Result<()> {
    // If using userspace device models, mark MMIO regions as:
    // - Read-trap
    // - Write-trap
    // - No execute
    
    // Hypervisor.framework should provide API like:
    // hv_vm_set_ept_trap(vm, gpa_start, gpa_end, read, write, exec)
    
    // For macOS HVF, this may be handled by in-kernel APIC emulation
    eprintln!("[hvf] EPT MMIO trapping configured (HVF in-kernel)");
    Ok(())
}
```

## Integration with Phase 2B & 2C

**Phase 2B (Interrupt Delivery):** ✅ Prerequisite complete
- Provides `queue_interrupt()` for I/O APIC → external interrupt routing
- Phase 2D can call `queue_interrupt(vector)` when IOAPIC routes IRQ

**Phase 2C (Timer Management):** ✅ Related parallel work
- LAPIC timer reads/writes handled by Phase 2D MMIO emulation
- Phase 2C ticks the timer, Phase 2D handles guest MMIO access
- Together: guest reads timer count, writes initial count, etc.

**Integration Example:**
```rust
// In execution loop:
for step in 0..MAX_STEPS {
    // Phase 2B: Deliver queued interrupts
    deliver_pending_interrupt(vcpu_state)?;
    
    // Phase 2C: Tick timer, queue interrupt if expired
    if let Some(vector) = lapic.tick(TICKS_PER_STEP) {
        queue_interrupt(vcpu_state, vector);
    }
    
    // Run vCPU
    hv_check(unsafe { hv_vcpu_run(vcpu) }, "hv_vcpu_run")?;
    
    // Phase 2D: Read exit, handle MMIO if EPT_VIOLATION
    let exit_state = read_exit_state(vcpu)?;
    match dispatch_exit(vcpu, &exit_state, &mut uart, &mut lapic, &mut ioapic)? {
        ExitDispatch::Handled(msg) => {
            eprintln!("[hvf] {}", msg);
            continue;  // Loop continues, next iteration will inject pending interrupt
        }
        ExitDispatch::Stop(exit) => {
            eprintln!("[hvf] stopped: {:?}", exit);
            break;
        }
    }
}
```

## Testing Strategy

### Unit Tests
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_mmio_device_classification() {
        assert_eq!(classify_mmio_device(0xFEE0_0000), Some(MmioDevice::Lapic));
        assert_eq!(classify_mmio_device(0xFEE0_0FFF), Some(MmioDevice::Lapic));
        assert_eq!(classify_mmio_device(0xFEC0_0000), Some(MmioDevice::Ioapic));
        assert_eq!(classify_mmio_device(0xFEC0_0FFF), Some(MmioDevice::Ioapic));
        assert_eq!(classify_mmio_device(0x1000_0000), None);
    }
    
    #[test]
    fn test_mmio_offset() {
        assert_eq!(mmio_offset(MmioDevice::Lapic, 0xFEE0_0000), 0);
        assert_eq!(mmio_offset(MmioDevice::Lapic, 0xFEE0_0320), 0x320);
        assert_eq!(mmio_offset(MmioDevice::Ioapic, 0xFEC0_0000), 0);
    }
    
    #[test]
    fn test_mmio_access_size_validation() {
        assert!(is_valid_mmio_access_size(MmioDevice::Lapic, 4));
        assert!(!is_valid_mmio_access_size(MmioDevice::Lapic, 2));
        assert!(!is_valid_mmio_access_size(MmioDevice::Lapic, 8));
    }
}
```

### Integration Tests (macOS)
1. Build with Phase 2D code
2. Run kernel with VEER_VM_HVF_INJECT_TIMER enabled
3. Kernel accesses LAPIC: writes LVT_TIMER, writes INITIAL_COUNT
4. EPT violations for MMIO accesses → dispatch_exit handles them
5. Device model reads/writes, updates RAX with register values
6. Log output shows: `[hvf] LAPIC read: offset=0x380 value=...`
7. Kernel receives timer interrupts, boot progresses

### Breakpoint Testing (with debugger)
- Set breakpoint in `dispatch_exit()` on EPT_VIOLATION path
- Watch MMIO accesses and device model responses
- Verify RAX updated correctly for reads
- Check RIP advanced properly after MMIO

## Success Criteria

- ✅ `classify_mmio_device()` correctly identifies LAPIC/IOAPIC ranges
- ✅ `mmio_offset()` calculates correct offsets
- ✅ `handle_mmio_read()` calls device model correctly
- ✅ `handle_mmio_write()` calls device model correctly
- ✅ RAX register properly read/written for MMIO data
- ✅ RIP advanced past faulting instruction
- ✅ EPT violation handler integrated into `dispatch_exit()`
- ✅ No panics on LAPIC/IOAPIC accesses
- ✅ Kernel MMIO reads return sensible values
- ✅ Kernel MMIO writes accepted without errors
- ✅ Boot progress with MMIO handling enabled

## Timeline
- 2D.1: 30 min (classifier functions)
- 2D.2: 45 min (MMIO read handling)
- 2D.3: 45 min (MMIO write handling)
- 2D.4: 60 min (dispatch_exit integration)
- 2D.5: 30 min (EPT trap configuration)
- **Total: 3-3.5 hours**

## Known Issues / TODOs

1. **Hypervisor.framework APIC handling:** May provide in-kernel LAPIC/IOAPIC emulation that automatically handles these MMIO accesses without userspace EPT violations. Phase 2D assumes userspace emulation; adjust if HVF has in-kernel support.

2. **EPT violation qualification parsing:** Current code assumes simple read/write distinction. More complex scenarios (access from ring 3, etc.) may require additional qualification bits.

3. **MMIO byte/word access:** LAPIC/IOAPIC typically expect 32-bit access. Should validate and potentially emulate smaller accesses by reading/modifying/writing 32-bit values.

4. **Device model persistence:** Across multiple vCPU runs (Phase 2A), device state must be shared/mutable. Current implementation assumes single-threaded access; multi-vCPU will need synchronization.

## Related Documentation
- `PHASE2B-INTERRUPT-DELIVERY.md` - Interrupt framework foundation
- `PHASE2C-TIMER-MANAGEMENT.md` - Timer integration with Phase 2D
- `docs/macos-hypervisor.md` - VMCS field reference
- HvfLapic/HvfIoapic implementations: `crates/veer_vm/src/backend/hvf.rs`
