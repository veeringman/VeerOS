# Phase 2B: Interrupt Delivery Framework (macOS HVF)

## Overview
Phase 2B implements the interrupt delivery infrastructure to route external interrupts (timer, I/O APIC, NMI, INIT) to vCPUs via VMCS interruption injection. This is foundational for timer management (2C), MMIO handling (2D), and halt/resume (2E).

## Architecture

### Interrupt Injection Mechanism

**VMCS Entry Interruption Info Field** (0x0000_4016):
```
Bits [31]      = Valid (1 = interrupt pending)
Bits [30:28]   = Undefined (0)
Bits [27]      = Error code valid (0 for most)
Bits [26:20]   = Undefined (0)
Bits [19:16]   = Undefined (0)
Bits [15:11]   = Undefined (0)
Bits [10:8]    = Delivery mode:
                 0 = External interrupt
                 2 = NMI (non-maskable)
                 5 = INIT
                 6 = SIPI (start-up)
Bits [7:0]     = Vector number (0-255)
```

**Example: External interrupt vector 32 (timer)**
```rust
let mut info = 0x8000_0000u64;  // Valid bit
info |= (32 as u64);             // Vector
// Result: 0x8000_0020
```

### Per-vCPU Interrupt State

Update `VcpuState` to handle multiple pending interrupts:
```rust
struct VcpuState {
    id: HvVcpuId,
    halted: bool,
    interrupt_queue: Vec<u8>,  // Multiple pending vectors
    nmi_pending: bool,         // Single NMI flag
    init_pending: bool,        // Single INIT flag
}
```

## Implementation Tasks

### Task 2B.1: Extend interrupt injection for NMI and INIT

**New functions:**
```rust
/// Inject NMI into vCPU (non-maskable, single queue)
fn inject_nmi(vcpu: HvVcpuId) -> Result<()> {
    let mut info = 0x8000_0000u64;
    info |= (2u64 << 8);  // NMI delivery mode
    // NMI has no vector, but still needs vector field = 0
    
    hv_check(
        unsafe { hv_vmx_vcpu_write_vmcs(vcpu, VMCS_CTRL_VMENTRY_INTERRUPTION_INFO, info) },
        "hv_vmx_vcpu_write_vmcs(INTERRUPTION_INFO for NMI)",
    )?;
    Ok(())
}

/// Inject INIT signal into vCPU (triggers AP startup protocol)
fn inject_init(vcpu: HvVcpuId) -> Result<()> {
    let mut info = 0x8000_0000u64;
    info |= (5u64 << 8);  // INIT delivery mode
    
    hv_check(
        unsafe { hv_vmx_vcpu_write_vmcs(vcpu, VMCS_CTRL_VMENTRY_INTERRUPTION_INFO, info) },
        "hv_vmx_vcpu_write_vmcs(INTERRUPTION_INFO for INIT)",
    )?;
    Ok(())
}
```

### Task 2B.2: Update VcpuState for multiple interrupt types

**Struct change:**
```rust
#[derive(Clone)]
struct VcpuState {
    id: HvVcpuId,
    halted: bool,
    interrupt_queue: VecDeque<u8>,  // External interrupt vectors
    nmi_pending: bool,
    init_pending: bool,
}
```

**In Drop trait for HvfVm:**
```rust
impl Drop for HvfVm {
    fn drop(&mut self) {
        for vcpu_state in &mut self.vcpus {
            // Clear pending interrupts to avoid re-injection on next boot
            vcpu_state.interrupt_queue.clear();
            vcpu_state.nmi_pending = false;
            vcpu_state.init_pending = false;
            
            let _ = unsafe { hv_vcpu_destroy(vcpu_state.id) };
        }
        // ... rest of cleanup
    }
}
```

### Task 2B.3: Add interrupt queue management functions

```rust
/// Queue an external interrupt for delivery when vCPU runs
fn queue_interrupt(vcpu_state: &mut VcpuState, vector: u8) {
    vcpu_state.interrupt_queue.push_back(vector);
}

/// Deliver oldest queued interrupt (FIFO priority)
fn deliver_queued_interrupt(vcpu: HvVcpuId, vcpu_state: &mut VcpuState) -> Result<Option<u8>> {
    if let Some(vector) = vcpu_state.interrupt_queue.pop_front() {
        inject_external_interrupt(vcpu, vector)?;
        return Ok(Some(vector));
    }
    Ok(None)
}

/// Deliver NMI if pending (higher priority than external)
fn deliver_nmi_if_pending(vcpu: HvVcpuId, vcpu_state: &mut VcpuState) -> Result<Option<()>> {
    if vcpu_state.nmi_pending {
        inject_nmi(vcpu)?;
        vcpu_state.nmi_pending = false;
        return Ok(Some(()));
    }
    Ok(None)
}

/// Check and clear INIT pending state
fn check_init_pending(vcpu_state: &mut VcpuState) -> bool {
    if vcpu_state.init_pending {
        vcpu_state.init_pending = false;
        return true;
    }
    false
}
```

### Task 2B.4: Update run_vcpu_placeholder execution loop

**Current code (Phase 1):**
```rust
fn run_vcpu_placeholder(vcpu: HvVcpuId) -> Result<()> {
    // Single run, exits immediately
    hv_check(unsafe { hv_vcpu_run(vcpu) }, "hv_vcpu_run")?;
    Ok(())
}
```

**Phase 2B update:**
```rust
fn run_vcpu_placeholder(vcpu: HvVcpuId, vcpu_state: &mut VcpuState) -> Result<()> {
    loop {
        // Deliver any pending interrupts before running
        if let Some(_) = deliver_nmi_if_pending(vcpu, vcpu_state)? {
            eprintln!("[hvf] NMI delivered to vCPU {}", vcpu);
        }
        
        if let Some(vector) = deliver_queued_interrupt(vcpu, vcpu_state)? {
            eprintln!("[hvf] External interrupt {} delivered to vCPU {}", vector, vcpu);
        }
        
        // Run vCPU until next exit
        let rc = unsafe { hv_vcpu_run(vcpu) };
        if rc != HV_SUCCESS {
            bail!("hv_vcpu_run failed: {} ({})", rc, hv_return_name(rc));
        }
        
        // Read exit state and classify
        let exit_state = get_exit_state(vcpu)?;
        let exit = classify_exit(vcpu, &exit_state)?;
        
        match exit {
            ExitReason::Hlt => {
                eprintln!("[hvf] vCPU {} halted", vcpu);
                vcpu_state.halted = true;
                
                // Wait for interrupt (TODO: Phase 2E)
                // For now, just break
                break;
            }
            ExitReason::Shutdown => {
                eprintln!("[hvf] vCPU {} shutdown", vcpu);
                break;
            }
            _ => {
                eprintln!("[hvf] Exit: {:?}", exit);
                // Phase 2D: Handle MMIO, CPUID, IO exits
                // For now, break to continue Phase 1 behavior
                break;
            }
        }
    }
    Ok(())
}
```

### Task 2B.5: Route device model interrupts through queue

**Integration example (for Phase 2C/2D):**
```rust
// In main execution loop (Phase 2C timer management):
pub fn run(cfg: VmConfig) -> Result<()> {
    // ... VM/vCPU setup ...
    
    let mut lapic = HvfLapic::new();
    
    for vcpu_state in &mut vcpu_states {
        loop {
            // Deliver pending interrupts
            // (Phase 2B helpers already in place)
            
            // Run vCPU
            unsafe { hv_vcpu_run(vcpu_state.id) }?;
            
            // Process timer tick (Phase 2C)
            if let Some(vector) = lapic.tick() {
                queue_interrupt(vcpu_state, vector);
            }
            
            // Process exit (Phase 2D)
            let exit = classify_exit(vcpu_state.id, &get_exit_state(vcpu_state.id)?)?;
            // ... handle exit ...
        }
    }
}
```

## Testing Strategy

### Unit Tests (Validation)
```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_interrupt_info_encoding() {
        // External interrupt vector 32
        let mut info = 0x8000_0000u64;
        info |= 32;
        assert_eq!(info, 0x8000_0020);
        
        // NMI
        let mut info = 0x8000_0000u64;
        info |= (2u64 << 8);
        assert_eq!(info, 0x8000_0200);
        
        // INIT
        let mut info = 0x8000_0000u64;
        info |= (5u64 << 8);
        assert_eq!(info, 0x8000_0500);
    }
    
    #[test]
    fn test_vcpu_state_interrupt_queue() {
        let mut vcpu = VcpuState {
            id: 0,
            halted: false,
            interrupt_queue: VecDeque::new(),
            nmi_pending: false,
            init_pending: false,
        };
        
        queue_interrupt(&mut vcpu, 32);
        queue_interrupt(&mut vcpu, 33);
        
        assert_eq!(vcpu.interrupt_queue.len(), 2);
        assert_eq!(vcpu.interrupt_queue[0], 32);  // FIFO order
    }
}
```

### Integration Tests (macOS only)
1. Boot kernel with interrupt delivery support
2. Verify timer interrupts are injected
3. Check vCPU halts and resumes on interrupt
4. Validate NMI/INIT delivery (if applicable)

## Success Criteria

- ✅ inject_external_interrupt/NMI/INIT functions compile and link on both Linux/macOS
- ✅ VcpuState supports multiple interrupt types
- ✅ Interrupt queue (FIFO) works for external interrupts
- ✅ run_vcpu_placeholder skeleton updated to call delivery functions
- ✅ VMCS fields correctly encoded (valid bit, delivery mode, vector)
- ✅ No panics when queueing/delivering interrupts

## Timeline
- 2 hours: Tasks 2B.1-2B.2 (injection functions + state struct)
- 1 hour: Task 2B.3 (queue management helpers)
- 1 hour: Task 2B.4 (execution loop update)
- 1 hour: Task 2B.5 (device model integration)
- **Total: ~5 hours** (enables 2C, 2D, 2E downstream)

## Related Components
- **HvfLapic**: Device model for LAPIC (provides timer interrupt vectors)
- **HvfIoapic**: Device model for I/O APIC (provides IRQ routing)
- **Phase 2C**: Timer management (calls queue_interrupt with timer vector)
- **Phase 2D**: MMIO handling (calls queue_interrupt for I/O APIC IRQs)
- **Phase 2E**: Halt/resume (uses interrupt_queue to wake halted vCPUs)
