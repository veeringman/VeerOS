# ESP32-C6 Interrupts & Timers — A Complete Guide

> **Audience**: developers new to bare-metal RISC-V or the ESP32-C6.
> Everything below was learned the hard way while bringing up VeerOS on a
> Seeed XIAO ESP32-C6 board.

---

## Table of Contents

1. [The Big Picture — Why Interrupts?](#1-the-big-picture--why-interrupts)
2. [RISC-V Interrupt Basics](#2-risc-v-interrupt-basics)
3. [ESP32-C6 Interrupt Architecture](#3-esp32-c6-interrupt-architecture)
4. [The Four Gates — Every Gate Must Be Open](#4-the-four-gates--every-gate-must-be-open)
5. [The Interrupt Matrix (INTMATRIX)](#5-the-interrupt-matrix-intmatrix)
6. [The PLIC (Platform-Level Interrupt Controller)](#6-the-plic-platform-level-interrupt-controller)
7. [The `mie` CSR — ESP32-C6's Hidden Requirement](#7-the-mie-csr--esp32-c6s-hidden-requirement)
8. [The Vector Table](#8-the-vector-table)
9. [SYSTIMER — The Kernel Heartbeat](#9-systimer--the-kernel-heartbeat)
10. [Putting It All Together — Boot to First Tick](#10-putting-it-all-together--boot-to-first-tick)
11. [What Happens When the Timer Fires](#11-what-happens-when-the-timer-fires)
12. [Common Pitfalls (and How We Hit Every One)](#12-common-pitfalls-and-how-we-hit-every-one)
13. [Register Quick-Reference](#13-register-quick-reference)

---

## 1. The Big Picture — Why Interrupts?

Without interrupts, the CPU would have to constantly ask "is there
anything to do?" in a tight loop (called **polling**). That wastes
power and makes it hard to share the CPU between multiple tasks.

**Interrupts** let hardware say "hey, something happened!" and force
the CPU to pause what it's doing, run a short handler function
(the **ISR** — Interrupt Service Routine), and then resume.

In VeerOS, the timer interrupt fires every **1 ms** and is the
heartbeat of the scheduler — it decides which task runs next.

```
┌──────────────┐        ┌──────────────┐        ┌──────────────┐
│   Task A     │  tick!  │  ISR (trap)  │  done   │   Task B     │
│  running     │───────▶│  scheduler   │───────▶│  running     │
│  ...         │        │  picks next  │        │  ...         │
└──────────────┘        └──────────────┘        └──────────────┘
```

---

## 2. RISC-V Interrupt Basics

The ESP32-C6 has a **RISC-V RV32IMC** CPU. RISC-V defines a handful
of **CSRs** (Control and Status Registers) that control interrupts:

| CSR        | Purpose                                              |
|------------|------------------------------------------------------|
| `mstatus`  | Global interrupt enable/disable (bit 3 = **MIE**)   |
| `mie`      | Per-interrupt-line enable mask                       |
| `mip`      | Per-interrupt-line pending status (read-only view)   |
| `mtvec`    | Address of the trap/vector table                     |
| `mcause`   | Reason for the most recent trap (interrupt or exception) |
| `mepc`     | PC (program counter) saved when trap occurred        |

**Key rule**: An interrupt only fires if ALL of these are true:
- `mstatus.MIE = 1` (global enable)
- The relevant bit in `mie` is set
- The relevant bit in `mip` is set (hardware drives this)
- Priority > threshold (managed by PLIC)

---

## 3. ESP32-C6 Interrupt Architecture

The ESP32-C6 does NOT use the standard RISC-V CLINT (Core Local
Interruptor). Instead it has a **three-stage pipeline**:

```
 ┌─────────────┐      ┌─────────────┐      ┌─────────┐      ┌─────┐
 │  Peripheral │      │  INTMATRIX   │      │  PLIC   │      │ CPU │
 │  (SYSTIMER, │─────▶│  (source →   │─────▶│ enable  │─────▶│ mip │
 │   WiFi, BLE,│      │   CPU int)   │      │ + prio  │      │ mie │
 │   etc.)     │      │              │      │ + thresh│      │ MIE │
 └─────────────┘      └─────────────┘      └─────────┘      └─────┘
   "I fired!"          "Route to           "Is it enabled    "Am I
                        line N"             & high enough     allowed
                                            priority?"        to take
                                                              it?"
```

Each stage is a **gate** — if any gate is closed, the interrupt
never reaches the CPU.

---

## 4. The Four Gates — Every Gate Must Be Open

For a peripheral interrupt (like the timer) to actually execute
your handler, **all four** of these must be configured:

### Gate 1: Interrupt Matrix (INTMATRIX)
Route the peripheral's **source number** to a **CPU interrupt line** (1–31).

### Gate 2: PLIC Enable + Priority
Enable the CPU interrupt line in the PLIC, and set its priority
above the threshold.

### Gate 3: `mie` CSR Bit
On the ESP32-C6 (unlike textbook RISC-V), you must set the
per-line bit in the `mie` CSR. This is NOT the standard MEIE/MTIE
— it's **bit N for CPU interrupt line N**.

### Gate 4: `mstatus.MIE`
The global interrupt enable. Set bit 3 of `mstatus`.

```
Gate 1 (INTMATRIX)  ──▶ "Source 57 → CPU int 1"
Gate 2 (PLIC)       ──▶ "CPU int 1: enabled, priority=1 > threshold=0"
Gate 3 (mie CSR)    ──▶ "mie bit 1 = 1"
Gate 4 (mstatus)    ──▶ "mstatus.MIE = 1"
                         ════════════════
                         ✅ Interrupt delivered!
```

If you miss **any one** gate, the hardware silently drops the
interrupt. No error, no warning — it just doesn't fire. This is
the single biggest source of confusion with ESP32-C6.

---

## 5. The Interrupt Matrix (INTMATRIX)

### What it does
The ESP32-C6 has over 70 peripheral interrupt **sources** (SYSTIMER,
WiFi, BLE, GPIO, UART, SPI, etc.). The CPU only has 32 **interrupt
lines** (0–31, where 0 is reserved for exceptions). The interrupt
matrix is a switchboard that routes any source to any CPU line.

### Base address
```
ESP32-C6:  0x6001_0000
ESP32-C3:  0x600C_2000   ← different!
```

### How to use it
Each source has a 32-bit register at `base + source_number * 4`.
Write the desired CPU interrupt line number (1–31) into it.

**Example** — Route SYSTIMER (source 57) to CPU interrupt line 1:
```
Address: 0x6001_0000 + 57 × 4 = 0x6001_00E4
Value:   1
```

In VeerOS code ([intc.rs](../crates/soc/esp32/src/intc.rs)):
```rust
pub fn map_source(&self, src: u16, cpu_int: u8) {
    let reg = INTC_BASE + (src as usize) * 4;
    mmio_write(reg, cpu_int as u32);
}
```

### Source numbers (key peripherals)

| Peripheral     | ESP32-C6 Source | ESP32-C3 Source |
|----------------|:-:|-|
| SYSTIMER (comp 0) | **57** | 37 |
| GPIO           | 28 | 28 |
| UART0          | 21 | 21 |
| WiFi MAC       | 1  | 1  |

> **Pitfall**: The source numbers differ between chips! We initially
> used source 7 (which is the C3 number) and couldn't figure out
> why the timer never fired.

---

## 6. The PLIC (Platform-Level Interrupt Controller)

### What it does
After the INTMATRIX routes a source to a CPU line, the **PLIC**
decides whether to actually signal the CPU. It provides:
- **Enable bits** — one per CPU interrupt line
- **Priority levels** — one per CPU interrupt line (0–15)
- **Threshold** — interrupts with priority ≤ threshold are blocked
- **Type** — level-triggered vs edge-triggered

### Base address
```
ESP32-C6:  0x2000_1000
```

### Key registers

| Register       | Offset | Width | Description |
|----------------|:------:|:-----:|-------------|
| MXINT_ENABLE   | 0x00   | 32    | Bit N = enable for CPU int N |
| MXINT_TYPE     | 0x04   | 32    | Bit N: 0=level, 1=edge |
| MXINT_CLEAR    | 0x08   | 32    | Write 1 to clear edge-pending |
| MXINT_PRI(N)   | 0x10 + 4×N | 4 | Priority for CPU int N (0–15) |
| MXINT_THRESH   | 0x90   | 8     | Global priority threshold |

### Setup in VeerOS

```rust
// 1. Set level-triggered (clear bit 1 in TYPE register)
let typ = mmio_read(PLIC_BASE + 0x04);
mmio_write(PLIC_BASE + 0x04, typ & !(1 << 1));

// 2. Clear any stale edge state
mmio_write(PLIC_BASE + 0x08, 1 << 1);

// 3. Enable CPU int 1 in PLIC
let en = mmio_read(PLIC_BASE + 0x00);
mmio_write(PLIC_BASE + 0x00, en | (1 << 1));

// 4. Set priority = 1 for CPU int 1
mmio_write(PLIC_BASE + 0x10 + 4*1, 1);

// 5. Set threshold = 0 (allow everything > 0)
mmio_write(PLIC_BASE + 0x90, 0);
```

> **Note**: On the ESP32-C3, the enable/priority/threshold registers
> live inside the INTMATRIX region (at offsets 0x104, 0x114, 0x190).
> The C6 moved them to a separate PLIC block. This is a major
> difference between the two chips.

---

## 7. The `mie` CSR — ESP32-C6's Hidden Requirement

This is the bug that took the longest to find.

### Standard RISC-V
In textbook RISC-V, `mie` has three bits:
- Bit 3: `MSIE` (software interrupt)
- Bit 7: `MTIE` (timer interrupt)
- Bit 11: `MEIE` (external interrupt)

### ESP32-C6 (Espressif's custom extension)
The ESP32-C6 uses `mie` as a **per-CPU-interrupt-line mask**:
- Bit 1 = CPU interrupt line 1
- Bit 2 = CPU interrupt line 2
- ...
- Bit 31 = CPU interrupt line 31

So for our SYSTIMER mapped to CPU int 1, we need `mie` bit 1 set:

```rust
// In enable_interrupt():
let mask = 1u32 << irq;  // irq = 1
core::arch::asm!("csrs mie, {0}", in(reg) mask);

// In disable_interrupt():
core::arch::asm!("csrc mie, {0}", in(reg) mask);
```

Without this, the PLIC asserts `mip` bit 1, but the CPU ignores it
because `mie` bit 1 is clear. The timer appears to fire (mip is
set) but the trap handler never executes.

> **Debugging clue**: If you read `mip` and see the bit set, but
> ISR count is zero, check `mie`.

---

## 8. The Vector Table

### Direct vs Vectored mode
RISC-V supports two trap delivery modes, controlled by `mtvec`:
- **Direct** (mode 0): All traps jump to a single address
- **Vectored** (mode 1): Exceptions go to base; interrupt N goes
  to `base + 4×N`

The ESP32-C6 PLIC **forces vectored mode** (mode bit must be 1).

### Vector table layout
The vector table is an array of 32 jump instructions. Each entry
must be exactly **4 bytes** (one `j` instruction). The table must
be **128-byte aligned**.

```asm
.balign  128          // 128-byte alignment required
.option push
.option norvc         // CRITICAL: disable compressed instructions

_veer_vector_table:
    j _veer_trap_entry   /* 0  exception → base+0 */
    j _veer_trap_entry   /* 1  CPU int 1 (timer) → base+4 */
    j _veer_trap_entry   /* 2  → base+8 */
    ...                  /* 3–31 */

.option pop
```

### Why `.option norvc`?
RISC-V "C" extension allows 2-byte compressed instructions.
A short `j` (jump within ±2 KiB) compresses to 2 bytes. But
vectored mode expects each slot to be exactly 4 bytes apart.
If a jump gets compressed to 2 bytes, the table entries shift
and the hardware jumps to the **wrong handler** — or into the
middle of an instruction.

`.option norvc` forces all instructions to be the full 4-byte
encoding.

### Installing the vector table

```rust
extern "C" { fn _veer_vector_table(); }

// base address | mode=1 (vectored)
let addr = (_veer_vector_table as usize & !0x3) | 1;
core::arch::asm!("csrw mtvec, {0}", in(reg) addr);
```

> **Pitfall**: If you forget `.option norvc`, the OS boots fine
> but jumps to garbage when an interrupt fires. The symptom is an
> illegal-instruction exception inside the vector table.

---

## 9. SYSTIMER — The Kernel Heartbeat

### What is it?
A hardware timer peripheral with a **52-bit free-running counter**
clocked at **16 MHz** (62.5 ns per tick). It has 3 comparators.
VeerOS uses comparator 0 in **periodic mode** for the 1 ms kernel tick.

### Base address
```
ESP32-C6:  0x6000_A000
ESP32-C3:  0x6002_3000   ← different!
```

### Key registers

| Register      | Offset | Description |
|---------------|:------:|-------------|
| CONF          | 0x00   | Clock enable (bit 0), unit 0 enable (bit 24) |
| UNIT0_OP      | 0x04   | Trigger counter snapshot (write bit 30) |
| TARGET0_HI    | 0x1C   | Comparator 0 high target value |
| TARGET0_LO    | 0x20   | Comparator 0 low target value |
| TARGET0_CONF  | 0x34   | Period mode (bit 30) + tick count (bits 25:0) |
| COMP0_LOAD    | 0x50   | Write 1 to apply comparator 0 config |
| UNIT0_VAL_HI  | 0x40   | Counter snapshot high word |
| UNIT0_VAL_LO  | 0x44   | Counter snapshot low word |
| INT_ENA       | 0x64   | Comparator 0 interrupt enable (bit 0) |
| INT_RAW       | 0x68   | Raw interrupt status |
| INT_CLR       | 0x6C   | Write 1 to clear interrupt |
| INT_ST        | 0x70   | Masked interrupt status |

### Configuring a 1 ms tick

At 16 MHz, 1 ms = 16,000 ticks. Setup steps:

```
1. Enable SYSTIMER clock:    CONF |= (1 << 0)
2. Enable unit 0 counter:    CONF |= (1 << 24)   "TARGET0_WORK_EN"
3. Set period mode + ticks:  TARGET0_CONF = (1 << 30) | 16000
4. Load the config:          COMP0_LOAD = 1
5. Enable the interrupt:     INT_ENA = 1
```

In VeerOS code ([systimer.rs](../crates/soc/esp32/src/systimer.rs)):
```rust
fn configure_tick(&self, period_us: u32) {
    let ticks = period_us * 16;  // 16 MHz → 16 ticks/µs

    // Enable clock + counter
    let conf = mmio_read(SYSTIMER_BASE + CONF_REG);
    mmio_write(SYSTIMER_BASE + CONF_REG, conf | (1 << 0) | (1 << 24));

    // Period mode with tick count
    mmio_write(SYSTIMER_BASE + TARGET0_CONF, (1 << 30) | ticks);

    // Apply config
    mmio_write(SYSTIMER_BASE + COMP0_LOAD, 1);

    // Enable interrupt
    mmio_write(SYSTIMER_BASE + INT_ENA, 1);
}
```

### The Re-Arm Trap (biggest gotcha)

**The ESP32-C6 SYSTIMER auto-disables the alarm after it fires.**

After a match, the hardware clears `TARGET0_WORK_EN` (CONF bit 24).
If your ISR only clears the interrupt flag (`INT_CLR = 1`), the
timer fires exactly **once** and never again.

You must **re-arm** the alarm by toggling `TARGET0_WORK_EN`:

```rust
fn clear_pending(&self) {
    // Step 1: Clear the interrupt flag
    mmio_write(SYSTIMER_BASE + INT_CLR, 1);

    // Step 2: Re-arm the alarm (toggle TARGET0_WORK_EN)
    let conf = mmio_read(SYSTIMER_BASE + CONF_REG);
    mmio_write(SYSTIMER_BASE + CONF_REG, conf & !(1 << 24));  // OFF
    mmio_write(SYSTIMER_BASE + CONF_REG, conf |  (1 << 24));  // ON
}
```

This matches what ESP-IDF does internally in `systimer_ll_enable_alarm()`.

> **Symptom if you forget**: The first timer ISR fires fine (ISR
> count = 1), but the count stays at 1 forever. The scheduler
> never preempts — whichever task runs first runs forever.

---

## 10. Putting It All Together — Boot to First Tick

Here's the complete sequence VeerOS follows during boot to get
the first timer interrupt firing:

```
_start (assembly)
  │
  ├── Disable interrupts:      csrci mstatus, 0x8
  ├── Set up kernel stack:     la sp, __stack_top
  ├── Zero BSS section
  └── Jump to _rust_start()
        │
        ├── Disable watchdogs (ROM bootloader enables them)
        ├── Wait for USB Serial/JTAG enumeration
        ├── Print boot banner
        │
        ├── ① Install vector table
        │     mtvec = &_veer_vector_table | 1   (vectored mode)
        │
        ├── ② Configure INTMATRIX
        │     INTMATRIX[57] = 1    (source 57 → CPU int 1)
        │
        ├── ③ Configure PLIC
        │     MXINT_TYPE:  bit 1 = 0  (level-triggered)
        │     MXINT_CLEAR: bit 1 = 1  (clear stale)
        │     MXINT_ENABLE: bit 1 = 1 (enable)
        │     MXINT_PRI[1] = 1        (priority 1)
        │     MXINT_THRESH = 0        (allow all > 0)
        │
        ├── ④ Set mie CSR
        │     csrs mie, (1 << 1)      (enable CPU int 1 in mie)
        │     (done inside enable_interrupt())
        │
        ├── ⑤ Configure SYSTIMER
        │     CONF: enable clock + TARGET0_WORK_EN
        │     TARGET0_CONF: period mode, 16000 ticks
        │     COMP0_LOAD: apply
        │     INT_ENA: enable
        │     ─── Timer is now counting! ───
        │
        ├── Create tasks (idle, shell, drivers)
        │     Set each task's mstatus = MPIE=1, MPP=M-mode
        │
        └── ⑥ Start scheduler → _veer_start_first_task()
              Loads first task context, `mret`:
              - MPIE → MIE (enables interrupts = Gate 4 opens)
              - Jump to task's entry point
              ─── First tick fires ~1 ms later ───
```

### Why `mstatus.MIE` opens last
Notice that global interrupts (mstatus.MIE) are off during all of
boot. They only turn on when `mret` executes at the very end.
This is by design — we don't want a timer interrupt during setup.

The trick: each task's saved `mstatus` has `MPIE=1`. When `mret`
copies MPIE → MIE, interrupts become enabled as part of entering
the first task.

```
INITIAL_MSTATUS = (1 << 7) | (3 << 11)
                   ↓            ↓
                 MPIE=1      MPP=M-mode

mret copies: MPIE → MIE   →  mstatus.MIE = 1  ✅
             MPP → mode    →  stays M-mode
```

---

## 11. What Happens When the Timer Fires

Every 1 ms, the SYSTIMER comparator matches and triggers this
chain:

```
SYSTIMER fires
  │
  ├── INTMATRIX routes source 57 → CPU int 1
  ├── PLIC sees int 1 enabled, priority(1) > threshold(0) → asserts mip[1]
  ├── CPU checks mie[1]=1, mstatus.MIE=1 → takes the trap
  │
  ├── Hardware automatically:
  │     mepc  ← current PC   (so we can return)
  │     mcause ← 0x80000001  (interrupt, code=1)
  │     mstatus: MIE → MPIE, MIE=0  (disable interrupts in handler)
  │     PC    ← mtvec.base + 4×1    (vector slot 1)
  │
  └── Vector table slot 1:  j _veer_trap_entry
```

### Inside `_veer_trap_entry` (assembly)

```
_veer_trap_entry:
  ┌── Save all 32 general-purpose registers onto the stack
  │   (136 bytes: 32 GPRs × 4 + mepc + mstatus)
  │
  ├── Save mepc and mstatus to the stack frame
  │
  ├── Call _veer_trap_dispatch(ctx)   ← Rust function
  │     │
  │     ├── Read mcause → is_interrupt=true, code=1
  │     ├── code==1 → handle_timer_tick()
  │     │     │
  │     │     ├── timer.clear_pending()
  │     │     │     ├── Write 1 to INT_CLR  (ack interrupt)
  │     │     │     └── Toggle TARGET0_WORK_EN (re-arm)
  │     │     │
  │     │     ├── sched.save_current_context()
  │     │     │     (copy CPU registers from stack into task's TCB)
  │     │     │
  │     │     ├── sched.tick()
  │     │     │     (decrement time slice, pick next task if expired)
  │     │     │
  │     │     ├── wake_sleepers()
  │     │     │     (wake tasks whose sleep timer expired)
  │     │     │
  │     │     ├── wake_poll_waiters()
  │     │     │     (wake tasks waiting on I/O events)
  │     │     │
  │     │     └── if need_switch:
  │     │           return &new_task.context  ← DIFFERENT context!
  │     │         else:
  │     │           return ctx                ← same context
  │     │
  │     └── Returns pointer to context to restore
  │
  ├── Restore mepc and mstatus from returned context
  ├── Restore all 32 GPRs from returned context
  └── mret
        ├── PC ← mepc         (resume the task)
        ├── MPIE → MIE        (re-enable interrupts)
        └── MPP → privilege   (back to M-mode)
```

### The Context Switch
The magic is in the return value of `_veer_trap_dispatch`:
- **No switch needed**: return the same `ctx` pointer → restore
  the interrupted task's registers → it resumes where it left off.
- **Switch needed**: return a pointer to the **new** task's saved
  context → restore different registers + different mepc → we
  "resume" into a completely different task.

The interrupted task doesn't know it was paused. Its registers
are safely saved in its Task Control Block (TCB) until it's
scheduled again.

---

## 12. Common Pitfalls (and How We Hit Every One)

### Pitfall 1: Wrong SYSTIMER base address
```
ESP32-C3: 0x6002_3000
ESP32-C6: 0x6000_A000   ← NOT the same!
```
Writing to the wrong address configures nothing. Timer never fires.

### Pitfall 2: Wrong interrupt source number
```
ESP32-C3 SYSTIMER source: 37
ESP32-C6 SYSTIMER source: 57   ← NOT the same!
```
If you use source 37, the INTMATRIX maps the wrong peripheral.

### Pitfall 3: Wrong INTMATRIX base address
```
ESP32-C3: 0x600C_2000
ESP32-C6: 0x6001_0000   ← NOT the same!
```
Same symptom — writes go to the wrong MMIO space.

### Pitfall 4: Missing `mie` CSR bit
The PLIC sets `mip[1]`, but if `mie[1]` is clear the CPU
ignores it. Standard RISC-V docs suggest only MEIE/MTIE/MSIE
matter — on the ESP32-C6 each CPU interrupt line has its own
`mie` bit.

### Pitfall 5: Timer fires only once (no re-arm)
The hardware clears `TARGET0_WORK_EN` after a match. You must
toggle it back on in every ISR, or you get exactly one tick.

### Pitfall 6: Compressed vector table entries
Without `.option norvc`, the `j` instructions can be 2 bytes
instead of 4. The hardware computes the handler address as
`base + 4×N`, landing in the wrong place.

### Pitfall 7: Task stacks in read-only memory
A Rust `static` (not `static mut`) goes into `.rodata`, which
on the ESP32-C6 maps to **DROM** (flash, read-only). Pushing
to the stack during a trap causes a store access fault. Use
`static mut` to place stacks in `.bss` (DRAM).

```
Bad:   static STACK: [u8; 4096] = [0; 4096];     // → DROM (read-only!)
Good:  static mut STACK: [u8; 4096] = [0; 4096];  // → BSS (DRAM, writable)
```

### Pitfall 8: USB Serial/JTAG doesn't auto-flush
The TX FIFO doesn't push bytes to the host until you set
`WR_DONE` (bit 0 of EP1_CONF at USB_BASE + 0x04). If you
use `print!`-style debugging in trap handlers, the output
sits in the FIFO and never appears. Use RAM-based counters
for debugging instead.

---

## 13. Register Quick-Reference

### INTMATRIX (Interrupt Matrix)

| Chip     | Base Address  |
|----------|:------------:|
| ESP32-C6 | `0x6001_0000` |
| ESP32-C3 | `0x600C_2000` |

```
Offset = source_number × 4
Value  = CPU interrupt line (1–31)
```

### PLIC (ESP32-C6 only — C3 embeds this in INTMATRIX)

| Register       | Address             | Bits |
|----------------|---------------------|------|
| MXINT_ENABLE   | `0x2000_1000`       | [31:0] enable per line |
| MXINT_TYPE     | `0x2000_1004`       | [31:0] 0=level, 1=edge |
| MXINT_CLEAR    | `0x2000_1008`       | [31:0] write-1-to-clear |
| MXINT_PRI(N)   | `0x2000_1010 + 4N`  | [3:0] priority |
| MXINT_THRESH   | `0x2000_1090`       | [7:0] threshold |

### SYSTIMER

| Chip     | Base Address  |
|----------|:------------:|
| ESP32-C6 | `0x6000_A000` |
| ESP32-C3 | `0x6002_3000` |

| Register      | Offset | Key bits |
|---------------|:------:|----------|
| CONF          | 0x00   | [0] CLK_EN, [24] TARGET0_WORK_EN |
| UNIT0_OP      | 0x04   | [30] trigger snapshot |
| TARGET0_HI    | 0x1C   | [19:0] high 20 bits of target |
| TARGET0_LO    | 0x20   | [31:0] low 32 bits of target |
| TARGET0_CONF  | 0x34   | [30] period mode, [25:0] period ticks |
| COMP0_LOAD    | 0x50   | Write 1 to apply |
| UNIT0_VAL_HI  | 0x40   | [19:0] counter high |
| UNIT0_VAL_LO  | 0x44   | [31:0] counter low |
| INT_ENA       | 0x64   | [0] comp0 interrupt enable |
| INT_RAW       | 0x68   | [0] comp0 raw status |
| INT_CLR       | 0x6C   | [0] write 1 to clear |
| INT_ST        | 0x70   | [0] masked status |

### RISC-V CSRs

| CSR      | Key bits for VeerOS |
|----------|---------------------|
| mstatus  | [3] MIE, [7] MPIE, [12:11] MPP |
| mie      | [N] = enable CPU int line N |
| mip      | [N] = pending CPU int line N |
| mtvec    | [31:2] base address, [1:0] mode (1=vectored) |
| mcause   | [31] interrupt flag, [30:0] code |
| mepc     | Saved PC on trap entry |

---

## Source Files

| File | Role |
|------|------|
| [`crates/soc/esp32/src/intc.rs`](../crates/soc/esp32/src/intc.rs) | INTMATRIX + PLIC driver |
| [`crates/soc/esp32/src/systimer.rs`](../crates/soc/esp32/src/systimer.rs) | SYSTIMER driver |
| [`crates/arch/src/riscv32.rs`](../crates/arch/src/riscv32.rs) | Vector table + trap entry/exit assembly |
| [`crates/kernel/xiao_esp32c6/src/trap.rs`](../crates/kernel/xiao_esp32c6/src/trap.rs) | Rust trap dispatcher |
| [`crates/kernel/xiao_esp32c6/src/main.rs`](../crates/kernel/xiao_esp32c6/src/main.rs) | Boot sequence + interrupt setup |
