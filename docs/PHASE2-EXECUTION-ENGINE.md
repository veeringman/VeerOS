# VeerOS Veer-VM Phase 2: Execution Engine for macOS HVF

**Status**: Implementation In Progress  
**Target Completion**: April 27, 2026  
**Backend**: VeerHV-Mac (macOS Hypervisor.framework)

---

## Phase 2 Overview

Phase 1 (Bring-up) established code-signing automation and basic VM lifecycle (create/destroy). Phase 2 implements the full execution engine with:

- **Multi-vCPU Support** — Parallel execution of multiple virtual processors
- **Interrupt Delivery Framework** — External interrupts, NMI, INIT signals
- **LAPIC & I/O APIC Timer Management** — PIT and timer-based guest synchronization
- **MMIO Trap Handling** — Device model integration for virtual device I/O
- **Guest Halt & Resume** — Power management and interrupt-driven awakening

---

## Architecture: Multi-vCPU Execution Loop

```
┌─────────────────────────────────────────────────────────────┐
│ hv_vm_create()                                              │
│ hv_vm_map() - guest memory                                  │
│ load_kernel() - kernel image                                │
└────────────────────────┬────────────────────────────────────┘
                         │
                         ▼
         ┌───────────────────────────────┐
         │ vCPU Pool Initialization       │
         │ for vcpu_id in 0..num_cpus:   │
         │   hv_vcpu_create()            │
         │   setup_initial_state()       │
         └───────────────┬───────────────┘
                         │
         ┌───────────────▼───────────────┐
         │ Main Execution Loop           │
         │ loop {                        │
         │   for each vcpu {             │
         │     hv_vcpu_run()             │
         │     handle_exit()             │
         │     check_interrupts()        │
         │   }                           │
         │ }                             │
         └──────────────┬────────────────┘
                        │
         ┌──────────────▼──────────────┐
         │ Exit Handling               │
         ├──────────────┬──────────────┤
         │              │              │
         │              ▼              │
         │    ┌─────────────────┐      │
         │    │ MMIO Access     │      │
         │    │ (Devices)       │      │
         │    └─────────────────┘      │
         │              │              │
         │              ▼              │
         │    ┌─────────────────┐      │
         │    │ I/O Port Access │      │
         │    │ (Serial, etc)   │      │
         │    └─────────────────┘      │
         │              │              │
         │              ▼              │
         │    ┌─────────────────┐      │
         │    │ Halt/Shutdown   │      │
         │    └─────────────────┘      │
         └──────────────────────────────┘
```

---

## Implementation Components

### 1. vCPU Pool Management

**Change from Phase 1:**
```rust
// Phase 1: Single vCPU
pub struct HvfVm {
    vcpu_id: Option<HvVcpuId>,
    // ...
}

// Phase 2: Multi-vCPU
pub struct HvfVm {
    vcpu_ids: Vec<HvVcpuId>,
    num_vcpus: usize,
    // ...
}
```

**vCPU Initialization Loop:**
```rust
for cpu_idx in 0..num_vcpus {
    hv_vcpu_create(&mut vcpu_id, ...)
    setup_vcpu_initial_state(vcpu_id, entry, ...)
    vcpu_ids.push(vcpu_id)
}
```

### 2. Interrupt Delivery Framework

**VMCS Entry Interruption Info Field:**
```rust
const VMCS_CTRL_VMENTRY_INTERRUPTION_INFO: u32 = 0x0000_4016;

// Encoding:
// [31] = Valid (1 = interrupt pending)
// [10:8] = Delivery mode (0=external, 2=NMI, 5=INIT)
// [7:0] = Vector (interrupt number)

fn inject_external_interrupt(vcpu: HvVcpuId, vector: u8) -> Result<()> {
    let mut info: u64 = 0x8000_0000; // Set valid bit
    info |= ((0u64) << 8);           // External interrupt delivery mode
    info |= (vector as u64);         // Vector
    
    unsafe {
        hv_vmx_vcpu_write_vmcs(vcpu, VMCS_CTRL_VMENTRY_INTERRUPTION_INFO, info)
    }
}
```

**Interrupt Sources:**
- **Device Interrupts** — From I/O APIC (routed from virtual devices)
- **Timer Interrupts** — From LAPIC timer expiration
- **NMI** — Non-maskable interrupt for debugging/monitoring
- **INIT** — Processor initialization signal

### 3. Timer Management

**LAPIC Timer Integration:**
```
Guest Timer Program
    │
    ├─ Write LVT_TIMER register
    ├─ Write TIMER_INIT (reload value)
    └─ Enable in SIVR
        │
        ▼
    LAPIC Timer Tick
        │
        ├─ Decrement TIMER_CURRENT
        ├─ Check if expired (== 0)
        └─ Generate Interrupt
            │
            ├─ Mask check: if masked, ignore
            ├─ Periodic vs One-shot
            └─ Inject vector into guest
```

**Device Routing (I/O APIC):**
```
PIT (Programmable Interval Timer)
    └─ IRQ2 (via I/O APIC)
        └─ Route to vCPU via external interrupt
           └─ Vector (typically 32+2=34)
```

### 4. MMIO Trap Handling

**MMIO Address Ranges:**
```
0xFEE0_0000-0xFEE0_0FFF  → LAPIC registers
0xFEC0_0000-0xFEC0_0FFF  → I/O APIC registers
0x3F8-0x3FF              → Serial port (I/O ports, not MMIO)
```

**Trap Flow:**
```rust
fn handle_exit(exit_reason: u32, vcpu: HvVcpuId) -> Result<()> {
    match exit_reason {
        HVF_EXIT_MMIO_READ => {
            let gpa = read_exit_qualification(vcpu)?;
            let value = route_mmio_read(gpa)?;
            write_register(vcpu, HV_X86_RAX, value)?;
        }
        HVF_EXIT_MMIO_WRITE => {
            let gpa = read_exit_qualification(vcpu)?;
            let value = read_register(vcpu, HV_X86_RAX)?;
            route_mmio_write(gpa, value)?;
        }
        _ => { /* other exits */ }
    }
}
```

### 5. Guest Halt & Resume

**Halt Instruction Handling:**
```
HLT instruction in guest
    │
    ▼
hv_vcpu_run() returns HVF_EXIT_HLT
    │
    ├─ Mark vCPU as halted
    ├─ Wait for interrupt (LAPIC timer, external IRQ)
    └─ Resume on interrupt
        │
        ├─ Inject interrupt vector
        └─ hv_vcpu_run() resumes
```

---

## Task Breakdown

### Phase 2A: Multi-vCPU Support (Current Sprint)
- [ ] Refactor HvfVm struct to use Vec<HvVcpuId>
- [ ] Create vCPU initialization loop
- [ ] Implement vCPU pool cleanup in Drop trait
- [ ] Basic multi-vCPU round-robin execution

### Phase 2B: Interrupt Delivery (Current Sprint)
- [ ] Implement external interrupt injection
- [ ] Add NMI injection support
- [ ] Implement INIT signal handling
- [ ] Add interrupt state tracking per vCPU

### Phase 2C: Timer Management (Current Sprint)
- [ ] Integrate LAPIC timer tick simulation
- [ ] Implement periodic timer mode
- [ ] Route timer interrupts to guest
- [ ] Add I/O APIC timer routing

### Phase 2D: MMIO Trap Handling (Current Sprint)
- [ ] Parse MMIO exit information from VMCS
- [ ] Route MMIO reads to device models
- [ ] Route MMIO writes to device models
- [ ] Implement LAPIC and I/O APIC MMIO handlers

### Phase 2E: Guest Halt & Resume (Current Sprint)
- [ ] Detect HLT exit
- [ ] Implement halt state per vCPU
- [ ] Resume on pending interrupts
- [ ] Add power-off detection

---

## Code Changes Required

### Files to Modify

1. **crates/veer_vm/src/backend/hvf.rs**
   - HvfVm struct (multi-vCPU)
   - run() function (execution loop)
   - New: handle_mmio_exit()
   - New: inject_interrupt()
   - New: check_and_deliver_interrupts()
   - Enhance: LAPIC timer tick()

2. **crates/veer_vm/src/backend/mod.rs**
   - Update documentation for Phase 2

3. **docs/macos-hypervisor.md**
   - Add Phase 2 execution engine details

### VMCS Fields Needed

```rust
// Exit information
const VMCS_RO_EXIT_QUALIFICATION: u32 = 0x0000_6400;
const VMCS_RO_EXIT_REASON: u32 = 0x0000_4002;
const VMCS_RO_VM_INSTRUCTION_ERROR: u32 = 0x0000_4400;

// Interrupt management
const VMCS_CTRL_VMENTRY_INTERRUPTION_INFO: u32 = 0x0000_4016;
const VMCS_GUEST_INTERRUPTIBILITY_STATE: u32 = 0x0000_4824;

// Activity state
const VMCS_GUEST_ACTIVITY_STATE: u32 = 0x0000_4826;
const VMCS_GUEST_ACTIVITY_STATE_ACTIVE: u64 = 0;
const VMCS_GUEST_ACTIVITY_STATE_HLT: u64 = 1;
const VMCS_GUEST_ACTIVITY_STATE_SHUTDOWN: u64 = 2;
const VMCS_GUEST_ACTIVITY_STATE_WAIT_SIPI: u64 = 3;
```

---

## Success Criteria

✅ **Multi-vCPU:**
- Create and manage up to 4 vCPUs
- Round-robin execution of all vCPUs
- Clean shutdown with all vCPU destruction

✅ **Interrupt Delivery:**
- Inject external interrupts (device IRQs)
- Deliver NMI for debugging
- Handle INIT signal for BSP election

✅ **Timer Management:**
- LAPIC timer fires on expiration
- Timer interrupts reach guest
- Periodic and one-shot modes work

✅ **MMIO Handling:**
- Guest reads/writes to LAPIC addresses
- Guest reads/writes to I/O APIC addresses
- Device models respond correctly

✅ **Halt & Resume:**
- Guest HLT instruction pauses vCPU
- Timer interrupt resumes vCPU
- Shutdown sequence respected

---

## Timeline Estimate

- **Multi-vCPU**: 2-3 hours
- **Interrupt Delivery**: 2-3 hours
- **Timer Management**: 2 hours
- **MMIO Handling**: 3-4 hours
- **Halt & Resume**: 1-2 hours
- **Testing & Validation**: 2-3 hours

**Total Phase 2**: ~15 hours work

---

## References

- [Hypervisor.framework VMX API](https://developer.apple.com/documentation/hypervisor/vmx)
- [Intel SDM: VMX Exit Processing](https://www.intel.com/content/dam/develop/external/us/en/documents/manual/64-ia-32-architectures-software-developers-manual-325462.pdf)
- [LAPIC Specification](https://en.wikibooks.org/wiki/X86_Assembly/Programmable_Interrupt_Controller)
- [VeerOS Architecture](./architecture.md)

---

## Phase 2 Status Board

```
Multi-vCPU Support      ████░░░░░░  40%
Interrupt Delivery      ░░░░░░░░░░   0%
Timer Management        ░░░░░░░░░░   0%
MMIO Trap Handling      ░░░░░░░░░░   0%
Halt & Resume           ░░░░░░░░░░   0%
─────────────────────────────────────
OVERALL PHASE 2         ████░░░░░░   8%
```

