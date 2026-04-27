# Phase 2C: Timer Management (macOS HVF)

## Overview
Phase 2C integrates the LAPIC timer simulation into the HVF execution loop, routing timer expirations as external interrupts to vCPUs. This enables guest kernel timer calibration and periodic task scheduling.

## Architecture

### Timer Components

**HvfLapic Device Model** (already implemented in hvf.rs):
```rust
struct HvfLapic {
    registers: [u32; 64],           // 256-byte MMIO register file
    timer_expiration: Option<u64>,  // TSC deadline for next timer tick
    irq: u8,                        // Interrupt vector for timer
}

impl HvfLapic {
    fn tick(&mut self, ticks: u64) -> Option<u8> {
        // Decrements timer count, returns interrupt vector on expiration
        // Returns None if timer not running or not yet expired
    }
    
    fn read_u32(&self, addr: u64) -> u32 { ... }
    fn write_u32(&mut self, addr: u64, val: u32) { ... }
}
```

### Timer Operation Flow

```
1. Guest writes to LAPIC_LVT_TIMER register
   - Sets timer vector, mode (periodic/one-shot), mask bit
   
2. Guest writes to LAPIC_INITIAL_COUNT
   - Specifies tick count before expiration
   
3. HVF execution loop calls lapic.tick(n)
   - Decrements internal counter by n
   - If counter <= 0, returns the interrupt vector
   
4. Execution loop calls queue_interrupt(vector)
   - Stage the interrupt for next vCPU run
   
5. run_vcpu_placeholder injects via VMCS
   - Sets INTERRUPTION_INFO with vector
   - vCPU executes and handles interrupt
   
6. Guest services timer interrupt
   - Reads EOI register, clears interrupt
   - LAPIC reloads counter for periodic mode
```

## Implementation Tasks

### Task 2C.1: Query LAPIC timer vector from device model

**New function:**
```rust
/// Phase 2C: Get the current LAPIC timer interrupt vector
fn get_lapic_timer_vector(lapic: &HvfLapic) -> u8 {
    // Extract vector from LAPIC_LVT_TIMER register (bits [7:0])
    let lvt_timer = lapic.read_u32(0xFEE00320);  // LAPIC_LVT_TIMER offset
    ((lvt_timer >> 0) & 0xFF) as u8
}

/// Phase 2C: Check if LAPIC timer is enabled (not masked)
fn is_lapic_timer_enabled(lapic: &HvfLapic) -> bool {
    let lvt_timer = lapic.read_u32(0xFEE00320);
    (lvt_timer & (1 << 16)) == 0  // Bit 16 = mask bit
}

/// Phase 2C: Check if LAPIC timer is in periodic mode
fn is_lapic_timer_periodic(lapic: &HvfLapic) -> bool {
    let lvt_timer = lapic.read_u32(0xFEE00320);
    (lvt_timer & (1 << 17)) != 0  // Bit 17 = periodic/one-shot
}
```

### Task 2C.2: Integrate timer tick into execution loop

**Update run_vcpu_placeholder():**
```rust
fn run_vcpu_placeholder(vcpu_state: &mut VcpuState) -> Result<()> {
    let vcpu = vcpu_state.id;
    // ... setup code ...
    
    let mut lapic = HvfLapic::default();
    // ... device model initialization ...
    
    const MAX_STEPS: usize = 32;
    const TICKS_PER_STEP: u64 = 1000;  // Simulate 1000 TSC ticks per vCPU_run
    
    for _step in 0..MAX_STEPS {
        // Phase 2B: Deliver any queued interrupts from previous iterations
        deliver_pending_interrupt(vcpu_state)?;
        
        // Phase 2C: Tick timer and queue interrupt if expired
        if let Some(vector) = lapic.tick(TICKS_PER_STEP) {
            if is_lapic_timer_enabled(&lapic) {
                queue_interrupt(vcpu_state, vector);
                eprintln!("[veer-vm] LAPIC timer expired, vector={}", vector);
            }
        }
        
        // Run vCPU
        hv_check(unsafe { hv_vcpu_run(vcpu) }, "hv_vcpu_run")?;
        
        // Phase 2D: Read and process exit
        let exit_state = read_exit_state(vcpu)?;
        match dispatch_exit(vcpu, &exit_state, &mut uart, &mut lapic, &mut ioapic)? {
            ExitDispatch::Handled(msg) => {
                eprintln!("[veer-vm] {}", msg);
                continue;
            }
            ExitDispatch::Stop(exit) => {
                eprintln!("[veer-vm] execution stopped, exit={:?}", exit);
                return Ok(());
            }
        }
    }
    Ok(())
}
```

### Task 2C.3: Integrate into main execution loop (Phase 2C+)

**Expanded main loop pattern (for future Phase 2D/2E):**
```rust
pub fn run(cfg: VmConfig) -> Result<()> {
    // ... VM setup ...
    
    let mut vcpu_states = vec![...];  // Multi-vCPU from Phase 2A
    
    // Device models
    let mut lapic = HvfLapic::default();
    let mut ioapic = HvfIoapic::default();
    
    'main_loop: loop {
        for vcpu_state in &mut vcpu_states {
            // Deliver any queued interrupts from previous cycles
            deliver_pending_interrupt(vcpu_state)?;
            
            // Tick timer, queue interrupt if expired
            if let Some(vector) = lapic.tick(1000) {
                if is_lapic_timer_enabled(&lapic) && !vcpu_state.halted {
                    queue_interrupt(vcpu_state, vector);
                }
            }
            
            // Phase 2E: Skip halted vCPUs (will be woken by interrupts)
            if vcpu_state.halted && vcpu_state.pending_interrupt.is_none() {
                continue;
            }
            
            // Run vCPU
            let rc = unsafe { hv_vcpu_run(vcpu_state.id) };
            if rc != HV_SUCCESS {
                bail!("vCPU {} run failed: {}", vcpu_state.id, hv_return_name(rc));
            }
            
            // Process exit
            let exit_state = read_exit_state(vcpu_state.id)?;
            let exit = classify_exit(vcpu_state.id, &exit_state)?;
            
            match exit {
                ExitReason::Hlt => {
                    vcpu_state.halted = true;
                    eprintln!("[hvf] vCPU {} halted", vcpu_state.id);
                }
                ExitReason::MmioRead { addr, len } => {
                    // Phase 2D: Handle MMIO accesses
                    let value = handle_mmio_read(&lapic, &ioapic, addr, len)?;
                    write_rax_low(vcpu_state.id, len, value)?;
                }
                ExitReason::Shutdown => {
                    eprintln!("[hvf] vCPU {} shutdown, exiting", vcpu_state.id);
                    break 'main_loop;
                }
                _ => {
                    eprintln!("[hvf] unhandled exit: {:?}", exit);
                }
            }
        }
    }
    Ok(())
}
```

### Task 2C.4: Test timer interrupt delivery

**Environment variables for testing:**
```bash
# Enable timer simulation
export VEER_VM_HVF_INJECT_TIMER=1

# Run veer-vm with timer enabled
./target/debug/veer-vm --kernel ./path/to/kernel --memory 128
```

**Expected behavior:**
1. Kernel boots and reaches LAPIC timer calibration routine
2. Kernel writes to LAPIC_INITIAL_COUNT
3. HVF loop calls lapic.tick() repeatedly
4. When counter expires, interrupt vector queued
5. vCPU receives interrupt, kernel services it
6. Log shows: `[veer-vm] LAPIC timer expired, vector=32`

### Task 2C.5: Implement one-shot vs periodic timer modes

**HvfLapic mode handling:**
```rust
fn tick(&mut self, ticks: u64) -> Option<u8> {
    // ... existing logic ...
    
    if self.timer_count <= 0 {
        let vector = self.get_timer_vector();
        
        // Check periodic mode (bit 17 of LVT_TIMER)
        if self.is_periodic() {
            // Reload counter from initial count
            self.timer_count = self.initial_count;
        } else {
            // One-shot: disable timer
            self.set_timer_disabled();
        }
        
        return Some(vector);
    }
    None
}
```

## Integration with Phase 2B

The Phase 2B interrupt delivery framework is the **prerequisite** for Phase 2C:

1. **queue_interrupt()** - Queues timer vector for delivery ✅ (Phase 2B)
2. **deliver_pending_interrupt()** - Injects queued timer interrupt ✅ (Phase 2B)
3. **inject_external_interrupt()** - VMCS interrupt setup ✅ (Phase 2B)

Phase 2C builds on this by:
- Calling `lapic.tick()` to detect expiration
- Using `queue_interrupt()` to stage the interrupt
- Relying on Phase 2B infrastructure for actual delivery

## Testing Strategy

### Unit Tests
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_lapic_timer_tick() {
        let mut lapic = HvfLapic::default();
        lapic.write_u32(0xFEE00320, 32 | (1 << 17));  // LVT_TIMER: vector 32, periodic
        lapic.write_u32(0xFEE00380, 1000);             // INITIAL_COUNT
        
        assert_eq!(lapic.tick(500), None);   // Not expired yet
        assert_eq!(lapic.tick(500), Some(32)); // Expires, returns vector
        assert_eq!(lapic.tick(0), None);      // Reloaded for periodic
    }
    
    #[test]
    fn test_lapic_timer_oneshot() {
        let mut lapic = HvfLapic::default();
        lapic.write_u32(0xFEE00320, 32);      // LVT_TIMER: vector 32, one-shot
        lapic.write_u32(0xFEE00380, 100);
        
        assert_eq!(lapic.tick(100), Some(32)); // Expires
        assert_eq!(lapic.tick(100), None);     // Disabled, no interrupt
    }
}
```

### Integration Tests (macOS)
1. Build with `scripts/build-mac.sh`
2. Run with environment variable: `VEER_VM_HVF_INJECT_TIMER=1`
3. Verify kernel detects timer interrupts during boot
4. Check "timer calibrated" messages in boot output
5. Validate interrupt handling in scheduler

## Success Criteria

- ✅ `get_lapic_timer_vector()` correctly reads LAPIC LVT_TIMER register
- ✅ `is_lapic_timer_enabled()` respects mask bit
- ✅ `is_lapic_timer_periodic()` detects mode from register
- ✅ Timer ticks integrated into execution loop (before vCPU_run)
- ✅ Expired timers queue interrupts via `queue_interrupt()`
- ✅ One-shot mode disables after expiration
- ✅ Periodic mode reloads counter
- ✅ No panics or segfaults in timer handling
- ✅ Kernel receives timer interrupts during boot

## Timeline
- 2C.1: 30 min (LAPIC timer query functions)
- 2C.2: 45 min (execution loop integration)
- 2C.3: 30 min (main loop patterns)
- 2C.4: 30 min (testing & validation)
- 2C.5: 30 min (mode handling)
- **Total: 2.5-3 hours**

## Related Documentation
- See `PHASE2B-INTERRUPT-DELIVERY.md` for interrupt framework details
- VMCS field reference: `docs/macos-hypervisor.md` 
- HvfLapic implementation: `crates/veer_vm/src/backend/hvf.rs` lines ~1100-1200
- Device model integration: Phase 2D (MMIO handling)
