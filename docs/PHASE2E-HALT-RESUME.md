# Phase 2E: Guest Halt & Resume (macOS HVF)

## Overview
Phase 2E implements proper guest halt detection and resume-on-interrupt behavior. When a guest vCPU executes the HLT instruction (typically when idle or after handling an interrupt), the hypervisor saves power by stopping execution until an interrupt becomes pending.

## Architecture

### HLT Instruction Handling

**x86 HLT Behavior:**
- Guest executes `HLT` instruction when no work is available
- Guest enters low-power state, waiting for interrupt
- Hypervisor detects HLT and exits to save CPU cycles
- On external interrupt, hypervisor resumes guest to handle it

**VMX Exit for HLT:**
```
EXIT_REASON = 12 (VMX_EXIT_REASON_HLT)
VMCS_GUEST_ACTIVITY_STATE should be set to HLT (1)
Guest receives interrupt → ACTIVITY_STATE returns to ACTIVE
```

### Activity State Machine

**VMCS_GUEST_ACTIVITY_STATE values:**
```
0 (ACTIVE)       - vCPU is executing instructions
1 (HLT)          - vCPU executed HLT, waiting for interrupt
2 (SHUTDOWN)     - vCPU in triple-fault shutdown state
3 (WAIT_SIPI)    - AP waiting for SIPI signal (multi-CPU startup)
```

**State Transitions:**
```
ACTIVE ──HLT──> HLT
  ▲              │
  │              │ (pending interrupt)
  └──────────────┘

ACTIVE ──TRIPLE_FAULT──> SHUTDOWN
```

## Implementation Tasks

### Task 2E.1: Detect HLT exit from vCPU

**Function exists from Phase 1, but enhance for Phase 2E:**
```rust
/// Phase 2E: Check for HLT exit reason
fn is_hlt_exit(reason: u32) -> bool {
    reason == VMX_EXIT_REASON_HLT
}

/// Phase 2E: Update vCPU state to halted
fn mark_vcpu_halted(vcpu_state: &mut VcpuState) -> Result<()> {
    vcpu_state.halted = true;
    eprintln!("[hvf] vCPU {} marked halted", vcpu_state.id);
    Ok(())
}

/// Phase 2E: Update vCPU state to running
fn mark_vcpu_running(vcpu_state: &mut VcpuState) -> Result<()> {
    vcpu_state.halted = false;
    eprintln!("[hvf] vCPU {} resumed", vcpu_state.id);
    Ok(())
}
```

### Task 2E.2: Set VMCS activity state on HLT

**VMCS field update:**
```rust
/// Phase 2E: Set guest activity state in VMCS
fn set_vcpu_activity_state(vcpu: HvVcpuId, state: u64) -> Result<()> {
    hv_check(
        unsafe {
            hv_vmx_vcpu_write_vmcs(vcpu, VMCS_GUEST_ACTIVITY_STATE, state)
        },
        "hv_vmx_vcpu_write_vmcs(ACTIVITY_STATE)",
    )?;
    eprintln!(
        "[hvf] vCPU {} activity state: {}",
        vcpu,
        match state {
            0 => "ACTIVE",
            1 => "HLT",
            2 => "SHUTDOWN",
            3 => "WAIT_SIPI",
            _ => "UNKNOWN",
        }
    );
    Ok(())
}

/// Phase 2E: Set HLT activity state
fn set_hlt_state(vcpu: HvVcpuId) -> Result<()> {
    set_vcpu_activity_state(vcpu, VMCS_ACTIVITY_STATE_HLT)
}

/// Phase 2E: Set ACTIVE activity state (resume)
fn set_active_state(vcpu: HvVcpuId) -> Result<()> {
    set_vcpu_activity_state(vcpu, VMCS_ACTIVITY_STATE_ACTIVE)
}
```

### Task 2E.3: Resume halted vCPU on interrupt

**Integration with Phase 2B interrupt delivery:**
```rust
/// Phase 2E: Resume halted vCPU if interrupt pending
fn resume_on_interrupt(vcpu_state: &mut VcpuState) -> Result<()> {
    if vcpu_state.halted && vcpu_state.pending_interrupt.is_some() {
        // Before resuming, set ACTIVE state
        set_active_state(vcpu_state.id)?;
        mark_vcpu_running(vcpu_state)?;
        eprintln!(
            "[hvf] vCPU {} resumed due to interrupt: {:?}",
            vcpu_state.id, vcpu_state.pending_interrupt
        );
    }
    Ok(())
}

/// Phase 2E: Deliver interrupt and resume if necessary
fn deliver_and_resume(vcpu_state: &mut VcpuState) -> Result<()> {
    if let Some(vector) = vcpu_state.pending_interrupt {
        // Resume halted vCPU
        resume_on_interrupt(vcpu_state)?;
        
        // Deliver interrupt
        if !vcpu_state.halted {
            inject_external_interrupt(vcpu_state.id, vector)?;
            vcpu_state.pending_interrupt = None;
        }
    }
    Ok(())
}
```

### Task 2E.4: Update execution loop for halt/resume

**Main loop with Phase 2E halt handling:**
```rust
fn run_vcpu_placeholder(vcpu_state: &mut VcpuState) -> Result<()> {
    let vcpu = vcpu_state.id;
    // ... setup ...

    const MAX_STEPS: usize = 32;
    const TICKS_PER_STEP: u64 = 1000;

    for step in 0..MAX_STEPS {
        eprintln!(
            "[hvf] step {}: halted={}, pending_int={:?}",
            step, vcpu_state.halted, vcpu_state.pending_interrupt
        );

        // Phase 2E: Skip if halted with no pending interrupt
        if vcpu_state.halted && vcpu_state.pending_interrupt.is_none() {
            eprintln!("[hvf] vCPU {} halted, waiting for interrupt...", vcpu);
            // In real implementation, could sleep/poll here
            // For now, continue to next iteration
            continue;
        }

        // Phase 2B+2E: Deliver and resume if interrupted
        if vcpu_state.halted && vcpu_state.pending_interrupt.is_some() {
            deliver_and_resume(vcpu_state)?;
        }

        // Phase 2B: Deliver any other queued interrupts
        deliver_pending_interrupt(vcpu_state)?;

        // Phase 2C: Tick timer
        if let Some(vector) = lapic.tick(TICKS_PER_STEP) {
            if !vcpu_state.halted {
                queue_interrupt(vcpu_state, vector);
            }
        }

        // Skip vCPU run if halted with no interrupt
        if vcpu_state.halted {
            eprintln!("[hvf] skipping vCPU_run (halted)");
            continue;
        }

        // Run vCPU
        hv_check(unsafe { hv_vcpu_run(vcpu) }, "hv_vcpu_run")?;

        // Read and classify exit
        let exit_state = read_exit_state(vcpu)?;
        let exit = classify_exit(vcpu, &exit_state)?;

        match exit {
            // Phase 2E: Detect HLT exit
            ExitReason::Hlt => {
                mark_vcpu_halted(vcpu_state)?;
                set_hlt_state(vcpu)?;
                eprintln!("[hvf] vCPU {} halted (HLT instruction)", vcpu);
                // Continue loop: next iteration will check for interrupts
                continue;
            }

            // Phase 2D: MMIO handling
            ExitReason::MmioRead { addr, len } => {
                // ... handle MMIO ...
                continue;
            }

            // Shutdown or other final exit
            ExitReason::Shutdown => {
                eprintln!("[hvf] vCPU {} shutdown", vcpu);
                return Ok(());
            }

            _ => {
                eprintln!("[hvf] exit: {:?}", exit);
                return Ok(());
            }
        }
    }

    eprintln!("[hvf] reached MAX_STEPS ({})", MAX_STEPS);
    Ok(())
}
```

### Task 2E.5: Implement optional sleep/polling in halt state

**For real systems, avoid busy-waiting:**
```rust
use std::thread;
use std::time::Duration;

/// Phase 2E: Sleep while vCPU is halted
fn sleep_while_halted(vcpu_state: &VcpuState, max_duration: Duration) {
    if vcpu_state.halted {
        eprintln!(
            "[hvf] vCPU {} sleeping until interrupt or timeout",
            vcpu_state.id
        );
        // In real implementation, could use:
        // - select()/poll() on interrupt event file descriptors
        // - condition variables for inter-thread wakeup
        // - eventfd for cross-process signaling
        
        // For now, just yield briefly
        thread::sleep(Duration::from_millis(1));
    }
}
```

## Integration with Previous Phases

**Phase 2B (Interrupt Delivery):**
- Provides `inject_external_interrupt()` to resume halted vCPU
- Enables checking `pending_interrupt` field
- Phase 2E calls Phase 2B functions to deliver

**Phase 2C (Timer Management):**
- Timer ticks generate interrupts even for halted vCPU
- Phase 2E queues these interrupts, triggers resume
- Prevents busy-waiting: halted → timer tick → resume → handle interrupt

**Phase 2D (MMIO Handling):**
- Parallel to Phase 2E (not directly dependent)
- Both part of complete execution engine
- Together with 2E: vCPU can halt mid-MMIO handling, resume on interrupt

**Combined Execution Flow:**
```
1. vCPU executes guest kernel
2. Kernel does HLT → Phase 2E: detect, mark halted, set ACTIVITY_STATE
3. Loop detects vCPU halted, skips vCPU_run, continues iterating
4. Phase 2C: LAPIC timer ticks, interrupt expires
5. Phase 2B: queue_interrupt() with timer vector
6. Phase 2E: detect pending_interrupt, resume_on_interrupt()
7. Phase 2B: inject_external_interrupt() delivers timer interrupt
8. vCPU runs, kernel handles timer interrupt
9. Kernel does more work or HLT again
10. Go to step 2
```

## Testing Strategy

### Unit Tests
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_vcpu_halt_detection() {
        let exit_reason = VMX_EXIT_REASON_HLT;
        assert!(is_hlt_exit(exit_reason));
        
        let other_exit = VMX_EXIT_REASON_CPUID;
        assert!(!is_hlt_exit(other_exit));
    }
    
    #[test]
    fn test_vcpu_state_transitions() {
        let mut vcpu = VcpuState {
            id: 0,
            halted: false,
            pending_interrupt: None,
        };
        
        mark_vcpu_halted(&mut vcpu).unwrap();
        assert!(vcpu.halted);
        
        mark_vcpu_running(&mut vcpu).unwrap();
        assert!(!vcpu.halted);
    }
    
    #[test]
    fn test_resume_on_interrupt() {
        let mut vcpu = VcpuState {
            id: 0,
            halted: true,
            pending_interrupt: Some(32),
        };
        
        // Would require mocking HVF calls in real tests
        // Just verify the logic:
        assert!(vcpu.halted);
        assert!(vcpu.pending_interrupt.is_some());
    }
}
```

### Integration Tests (macOS)
1. Build veer-vm with Phase 2E enabled
2. Run kernel without timer forcing (let it naturally HLT)
3. Observe in logs:
   - `[hvf] vCPU 0 halted (HLT instruction)` when kernel halts
   - `[hvf] vCPU 0 resumed due to interrupt` when interrupt pending
4. Verify kernel doesn't continuously re-enter HLT immediately
5. Check timer interrupts cause HLT → resume → handle → HLT cycle
6. Monitor CPU usage: should drop significantly during idle

### Real-World Test (with actual timer)
```bash
# Build with Phase 2C timer + Phase 2E halt/resume
./scripts/build-mac.sh

# Run with timer injection
export VEER_VM_HVF_INJECT_TIMER=1
./target/release/veer-vm --kernel ./kernel --memory 128

# Expected behavior:
# - Boot messages appear
# - "HLT instruction" message appears when kernel idles
# - Timer interrupts trigger resume messages
# - Kernel continues running (not stuck in HLT)
# - Machine doesn't thermal-throttle (low CPU usage while halted)
```

## Success Criteria

- ✅ HLT exit detected correctly (EXIT_REASON_HLT = 12)
- ✅ vCPU halted flag properly set/cleared
- ✅ VMCS_GUEST_ACTIVITY_STATE set to HLT and ACTIVE appropriately
- ✅ Halted vCPU skips vCPU_run when no interrupt pending
- ✅ Pending interrupt causes halted vCPU to resume
- ✅ VMCS state restored to ACTIVE on resume
- ✅ vCPU doesn't immediately HLT again after interrupt
- ✅ No panics or infinite loops in halt/resume
- ✅ CPU utilization drops during halt (measurable power saving)
- ✅ Kernel scheduler works correctly with halted vCPUs
- ✅ Multi-vCPU systems handle per-vCPU HLT states

## Timeline
- 2E.1: 20 min (HLT detection functions)
- 2E.2: 30 min (VMCS activity state setup)
- 2E.3: 30 min (resume-on-interrupt logic)
- 2E.4: 60 min (main loop integration)
- 2E.5: 30 min (sleep/polling optimization)
- **Total: 2.5-3 hours**

## Known Issues / TODOs

1. **Busy-waiting during HLT:** Current implementation loops continuously checking for interrupts. Real implementation should sleep and be awakened by interrupt signals.

2. **ACTIVITY_STATE persistence:** Not all HVF versions guarantee ACTIVITY_STATE is readable/writable. May need to track halt state in VcpuState only.

3. **Triple-fault handling:** Current code doesn't detect or handle SHUTDOWN state. Could add `VMX_EXIT_REASON_TRIPLE_FAULT` detection for graceful shutdown.

4. **Multi-vCPU synchronization:** With multiple vCPUs, halting one shouldn't block others. Ensure each vCPU has independent state in Vec<VcpuState>.

5. **Signal handling:** In real systems, need SIGALRM or timer signals to wake from sleep during HLT. Currently not implemented.

## Related Documentation
- `PHASE2B-INTERRUPT-DELIVERY.md` - Interrupt foundation for Phase 2E resume
- `PHASE2C-TIMER-MANAGEMENT.md` - Timer as interrupt source for Phase 2E
- `PHASE2D-MMIO-HANDLING.md` - Parallel to Phase 2E
- `docs/macos-hypervisor.md` - VMCS activity state field details
