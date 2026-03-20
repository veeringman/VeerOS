# WiFi (Network) Bringup — ESP32-C6 on VeerOS

> **Board**: Seeed XIAO ESP32-C6  
> **MAC**: `e4:b3:23:b5:dd:28`  
> **SoC**: ESP32-C6 (RISC-V RV32IMC, single-core, 160 MHz)  
> **WiFi**: 802.11ax (WiFi 6), 2.4 GHz  
> **Blobs**: esp-wifi-sys v0.8.1 (libpp.a, libnet80211.a, libphy.a, libcore.a)  
> **Reference**: esp-wifi v0.15.1  
> **Status**: `register_chipv7_phy()` blocks — see §12  

---

## Table of Contents

1.  [Overview — How WiFi Works on ESP32-C6](#1-overview)
2.  [Architecture & Crate Layout](#2-architecture--crate-layout)
3.  [The Blob Integration Model](#3-the-blob-integration-model)
4.  [The OSI Adapter (wifi_osi_funcs_t)](#4-the-osi-adapter)
5.  [Interrupt Pipeline for WiFi](#5-interrupt-pipeline-for-wifi)
6.  [WiFi ISR Dispatch](#6-wifi-isr-dispatch)
7.  [Scheduler Task Integration (ppTask)](#7-scheduler-task-integration-pptask)
8.  [PHY Calibration & register_chipv7_phy](#8-phy-calibration)
9.  [Memory / Heap for Blobs](#9-memory--heap-for-blobs)
10. [WiFi Init Flow (Steps 1-A)](#10-wifi-init-flow)
11. [WiFi Connect Flow (Steps B-D)](#11-wifi-connect-flow)
12. [Current Blocking Point — Root Cause Analysis](#12-current-blocking-point)
13. [Configuration System (/etc/net/wifi)](#13-configuration-system)
14. [Trace Marker Reference](#14-trace-marker-reference)
15. [Key Addresses & Symbols](#15-key-addresses--symbols)
16. [Build, Flash & Monitor](#16-build-flash--monitor)
17. [Serial Output Decoding](#17-serial-output-decoding)
18. [Next Steps & Hypotheses](#18-next-steps--hypotheses)
19. [Reference: esp-wifi Comparison](#19-reference-esp-wifi-comparison)
20. [Appendix: OSI Function Table Layout](#20-appendix-osi-function-table)

---

## 1. Overview

The ESP32-C6's WiFi radio is controlled by proprietary firmware "blobs"
from Espressif — precompiled static libraries (libpp.a, libnet80211.a,
libphy.a, libcore.a). These blobs were designed to run on FreeRTOS and
call through a well-defined **OS abstraction layer** called the
`wifi_osi_funcs_t` function pointer table.

VeerOS replaces FreeRTOS entirely. To make WiFi work, we implement
every function in `wifi_osi_funcs_t`, bridging blob expectations
(mutexes, queues, semaphores, task creation, interrupts, timers,
memory allocation) to VeerOS primitives.

**Data flow (TX/RX):**

```
Application
  │
  ▼
WifiManager.send(buf)
  │
  ▼
esp_wifi_internal_tx()     ← blob function
  │
  ▼
WiFi MAC hardware → radio → air
                               │
                               ▼ (receives frame)
WiFi MAC interrupt → ISR → recv_cb_sta() → RX ring buffer
                                                │
                                                ▼
                                          Application reads RX
```

---

## 2. Architecture & Crate Layout

```
crates/
  arch/src/lib.rs              # NetworkDevice trait, NetMedium enum
  soc/esp32/src/
    wifi.rs                    # WifiManager, Esp32Wifi driver, RX ring buffer
    wifi_os_adapter.rs         # OSI adapter: all FreeRTOS-like primitives
    c_stubs.rs                 # C library stubs (memcpy, strlen, etc.)
    heap.rs                    # Simple first-fit allocator for blobs (128 KiB)
    modem.rs                   # MODEM_LPCON clock/reset, eFuse MAC, register maps
    systimer.rs                # now_us() microsecond timer
  kernel/xiao_esp32c6/src/
    main.rs                    # wifi_driver_task(), /etc/net/wifi creation, boot
    trap.rs                    # Interrupt dispatch → wifi_isr_dispatch()
  userlib/src/
    config.rs                  # Key=value config parser (no_std)
```

**Key types:**

| Type | Location | Purpose |
|------|----------|---------|
| `WifiManager` | wifi.rs | High-level config + state machine |
| `Esp32Wifi` | wifi.rs | Low-level driver: init, connect, scan, TX/RX |
| `WifiNetProxy` | main.rs | Implements `NetworkDevice` by forwarding to global `WIFI` |
| `OSI_FUNCS` | wifi_os_adapter.rs | `wifi_osi_funcs_t` const with all function pointers |
| `SimpleQueue` | wifi_os_adapter.rs | Ring-buffer queue for blob IPC |
| `SimpleSem` | wifi_os_adapter.rs | Semaphore/mutex (supports recursive) |
| `SimpleEventGroup` | wifi_os_adapter.rs | Bitmask event group |
| `SoftTimer` | wifi_os_adapter.rs | Software timer (polled, ets_timer) |
| `BlobTask` | wifi_os_adapter.rs | Slot for blob-spawned tasks |

---

## 3. The Blob Integration Model

The Espressif blobs expect a FreeRTOS-like environment. Each blob
function does its work and calls back into our OS through the OSI table.

**Libraries and what they provide:**

| Library | Contents |
|---------|----------|
| **libpp.a** | `ppTask` (main WiFi processing task), `ic_set_interrupt_handler`, packet processing |
| **libnet80211.a** | IEEE 802.11 state machine, `wifi_hw_start`, `wifi_start_process`, scan, connect |
| **libphy.a** | `register_chipv7_phy` (RF calibration), `phy_wakeup_init`, PHY parameter config |
| **libcore.a** | `g_log_level`, `g_misc_nvs`, NVS stubs, miscellaneous ROM-era wrappers |

**Global variables the blobs expect:**

| Symbol | Address | Purpose |
|--------|---------|---------|
| `g_osi_funcs_p` | `0x4087FF6C` | ROM data pointer → our OSI table |
| `g_wifi_osi_funcs` | (BSS) | The actual `wifi_osi_funcs_t` struct |
| `g_wifi_feature_caps` | (BSS) | Feature capability bitmask |
| `WIFI_EVENT` | (BSS) | Event base string pointer |
| `pp_task_hdl` | `0x4087FF40` | ppTask handle (written by `task_create_pinned_to_core`) |
| `s_wifi_queue` | `0x4087FF44` | WiFi command queue handle |
| `g_ic` | `0x40857298` | WiFi internal context structure |

---

## 4. The OSI Adapter

The `wifi_osi_funcs_t` table has ~100 function pointers. Our
implementation lives in `crates/soc/esp32/src/wifi_os_adapter.rs`.

**Categories of functions we implement:**

### 4.1 Interrupt Management

| Function | Our Implementation |
|----------|--------------------|
| `set_intr(cpu_no, source, intr_num, prio)` | Traces 'I'; ignores blob routing (would collide with SYSTIMER on CPU INT 1) |
| `clear_intr(source, intr_num)` | Clears PLIC pending for CPU INT 2 & 3 |
| `set_isr(n, handler, arg)` | Stores handler in `WIFI_ISR_FN`/`WIFI_ISR_ARG` globals |
| `ints_on(mask)` | Enables bits in PLIC enable register + `mie` CSR |
| `ints_off(mask)` | Disables bits in PLIC enable register + `mie` CSR |

**Critical design decision**: The blob calls `set_intr` requesting CPU INT 1
for WiFi MAC. But CPU INT 1 is already used by SYSTIMER (the kernel tick).
We ignore the blob's request and pre-route WiFi interrupts to CPU INT 2 & 3
in `setup_wifi_interrupts()`.

### 4.2 Task Management

| Function | Our Implementation |
|----------|--------------------|
| `task_create_pinned_to_core` | Allocates a `BlobTask` slot, calls `SYS_SPAWN` to create a real scheduler task |
| `task_create` | Forwards to `task_create_pinned_to_core` with core 0 |
| `task_delete` | No-op (no SYS_KILL yet) |
| `task_delay(ticks)` | `SYS_SLEEP(ticks)` |
| `task_get_current_task` | `SYS_TASK_ID` |

### 4.3 Synchronization

| Function | Our Implementation |
|----------|--------------------|
| `semphr_create(max, init)` | Allocates `SimpleSem` with count=init, max=max |
| `semphr_take(h, timeout)` | Spin-wait with `poll_timers()` + `SYS_SLEEP(1)` per iteration |
| `semphr_give(h)` | Increments count (up to max) |
| `mutex_create` | `alloc_sem(1, 1)` — binary semaphore, initially unlocked |
| `recursive_mutex_create` | `alloc_recursive_mutex()` — tracks owner + recursion depth |
| `mutex_lock` | Checks ownership for recursive re-entry, else `semphr_take` |
| `mutex_unlock` | Decrements recursion; only gives semaphore at recursion=0 |

**Recursive mutex details**: The `SimpleSem` struct has fields:
- `recursive: bool` — distinguishes from regular mutex
- `owner: usize` — task ID of current holder (0 = unowned)
- `recursion: u32` — nesting depth

### 4.4 Queue

| Function | Our Implementation |
|----------|--------------------|
| `queue_create(len, item_size)` | Allocates `SimpleQueue` ring buffer (max 2048 bytes) |
| `queue_send(h, item, timeout)` | Copies item into ring, advances tail |
| `queue_recv(h, item, timeout)` | Blocking: spin-wait with `poll_timers()` + `SYS_SLEEP(1)` |
| `queue_msg_waiting(h)` | Returns count of items in queue |

The `SimpleQueue` struct starts with `pc_head` and `pc_write_to` fields
mimicking FreeRTOS `QueueHandle_t` layout, because blobs dereference
queue handles at offset 0.

### 4.5 Event Groups

| Function | Our Implementation |
|----------|--------------------|
| `event_group_create` | Allocates `SimpleEventGroup` with bits=0 |
| `event_group_set_bits(h, bits)` | OR-in bits |
| `event_group_wait_bits(h, bits, clear, all, timeout)` | Spin-wait checking mask match |
| `event_group_clear_bits(h, bits)` | AND-NOT bits |

### 4.6 Timers (ets_timer)

| Function | Our Implementation |
|----------|--------------------|
| `timer_setfn(ptimer, func, arg)` | Stores callback in `SoftTimer` slot (hash-mapped from blob pointer) |
| `timer_arm(ptimer, ms, repeat)` | Sets `next_fire = now + ms*1000`, `active = true` |
| `timer_arm_us(ptimer, us, repeat)` | Same but microsecond precision |
| `timer_disarm(ptimer)` | Sets `active = false` |

Timers are polled from `poll_timers()`, called from:
- `semphr_take()` while waiting
- `queue_recv()` while waiting
- `event_group_wait_bits()` while waiting
- `wifi_driver_task()` main loop via connect path

### 4.7 PHY & Radio

| Function | Our Implementation |
|----------|--------------------|
| `phy_enable` | Full path: `enable_all_clocks()` → `register_chipv7_phy()` → set `PHY_CALIBRATED` |
| `phy_disable` | Sets `PHY_ENABLED = false` |
| `wifi_clock_enable` | `modem::enable_all_clocks()` |
| `wifi_clock_disable` | No-op (other radios may need clocks) |
| `wifi_reset_mac` | `modem::reset_all_modems()` |

### 4.8 Memory

| Function | Our Implementation |
|----------|--------------------|
| `malloc` / `free` / `realloc` / `calloc` | heap.rs: 128 KiB first-fit free-list allocator |
| `wifi_malloc` / `wifi_zalloc` etc. | Same allocator |
| `get_free_heap_size` | Returns `HEAP_SIZE - ALLOCATED` |

### 4.9 NVS (Non-Volatile Storage)

All NVS functions return `-1` (failure). The blobs handle NVS
unavailability gracefully — they skip persistence.

### 4.10 Coexistence

All coex functions are no-ops returning success (0). Single-radio
operation doesn't need coexistence arbitration.

---

## 5. Interrupt Pipeline for WiFi

The ESP32-C6 has a three-stage interrupt pipeline. For WiFi, we
configure it as follows:

```
Peripheral Sources          INTMATRIX              PLIC                CPU
───────────────────    ─────────────────    ─────────────────    ──────────
WIFI_MAC (src 0)    →  CPU INT 2           enabled, prio 1      mie bit 2
WIFI_PWR (src 2)    →  CPU INT 3           enabled, prio 1      mie bit 3
WIFI_BB  (src 3)    →  CPU INT 31          NOT enabled           (sink)
MODEM_TIMEOUT(34)   →  CPU INT 31          NOT enabled           (sink)
SYSTIMER (src 57)   →  CPU INT 1           enabled, prio 1      mie bit 1
```

**Register addresses and configuration:**

| Register | Address | What we write |
|----------|---------|---------------|
| INTMATRIX source 0 | `0x60010000 + 0*4` | 2 (→ CPU INT 2) |
| INTMATRIX source 2 | `0x60010000 + 2*4` | 3 (→ CPU INT 3) |
| INTMATRIX source 3 | `0x60010000 + 3*4` | 31 (sink) |
| INTMATRIX source 34 | `0x60010000 + 34*4` | 31 (sink) |
| PLIC priority INT 2 | `0x20001010 + 2*4` | 1 |
| PLIC priority INT 3 | `0x20001010 + 3*4` | 1 |
| PLIC enable | `0x20001000` | OR in bits 2 & 3 |
| PLIC type | `0x20001004` | Clear bits 2 & 3 (level-triggered) |
| PLIC clear | `0x20001008` | Set bits 2 & 3 (clear stale) |
| mie CSR | — | OR in `(1<<2) \| (1<<3)` |

This is done in `setup_wifi_interrupts()`, called during WiFi init
step 3b (after OSI table setup, before `esp_wifi_init_internal`).

---

## 6. WiFi ISR Dispatch

When a WiFi interrupt fires, the CPU takes the trap:

```
_veer_trap_entry (assembly)
  → _veer_trap_dispatch(ctx)       [trap.rs]
    → handle_interrupt(ctx, code)
      → match code:
          1 → handle_timer_tick()   [SYSTIMER]
          _ → wifi_isr_dispatch(code) [wifi_os_adapter.rs]
```

`wifi_isr_dispatch(cpu_int)`:
1. Checks if `cpu_int == 2` (WIFI_MAC) or `cpu_int == 3` (WIFI_PWR)
2. Reads `WIFI_ISR_FN` — the handler stored by `set_isr()`
3. If non-null, calls `handler(WIFI_ISR_ARG)`
4. Returns `true` to indicate it was handled

**The ISR handler is registered by `ic_set_interrupt_handler()`** inside
`libpp.a`. This function lives at address `0x42261FA8` and calls through
our OSI table at offsets 8 (`set_intr`), 16 (`set_isr`), and 20 (`ints_on`).

**Current status**: `ic_set_interrupt_handler` is never reached because
it's called AFTER `phy_enable` in the `wifi_hw_start` call chain, and
`phy_enable` blocks (see §12).

---

## 7. Scheduler Task Integration (ppTask)

The blobs call `task_create_pinned_to_core` to spawn `ppTask` — the main
WiFi processing loop in libpp.a. Our adapter:

1. Finds a free `BlobTask` slot (4 available)
2. Stores the blob's function pointer and parameter
3. Calls `SYS_SPAWN` (ecall a7=5) with:
   - entry = `blob_task_entryN` trampoline function
   - stack = 8 KiB stack allocated in `BlobTask.stack`
   - priority = blob's requested priority
4. Records the kernel task ID in `BlobTask.task_id`

**Why SYS_SPAWN?** Earlier we used cooperative polling (`poll_blob_task`
with `setjmp`/`longjmp`). This caused deadlocks because blob tasks need
to block on semaphores/queues indefinitely. With real scheduler tasks,
ppTask can sleep while waiting for commands without blocking the wifi-drv
thread.

**`ppTask` execution flow:**
```
ppTask starts
  → blocks on queue_recv(s_wifi_queue)     [waiting for commands]
  → receives START command (from esp_wifi_start)
  → calls wifi_start_process()
    → calls wifi_hw_start(mode=3 for STA)
      → ieee80211_set_hmac_stop(0)
      → wifi_apb80m_request()              [no-op]
      → wifi_clock_enable()                [modem::enable_all_clocks]
      → wifi_rtc_disable_iso()             [no-op]
      → phy_enable()                       [*** BLOCKS HERE ***]
      → coex_enable()                      [never reached]
      → wifi_reset_mac()                   [never reached]
      → ic_mac_init()                      [never reached]
      → chm_init()                         [never reached]
      → ic_set_interrupt_handler()         [never reached]
  → would set event bits to unblock esp_wifi_start caller
```

---

## 8. PHY Calibration

The RF transceiver requires calibration before use. This is done by
`register_chipv7_phy()` from libphy.a.

**Our `phy_enable()` implementation:**

```
phy_enable() called by blob
  ├─ if PHY_ENABLED → skip (already on)
  │
  ├─ enable_all_clocks()
  │
  ├─ if !PHY_CALIBRATED:
  │     register_chipv7_phy(&PHY_INIT_DATA, &PHY_CAL_DATA, PHY_RF_CAL_FULL)
  │     PHY_CALIBRATED = true        ← never reached (blocks)
  │
  └─ else:
        phy_wakeup_init()            ← fast path for subsequent enables
```

**PHY init data**: 128-byte parameter array matching esp-wifi defaults
(20 dBm max TX power). Stored in const `PHY_INIT_DATA`.

**Calibration data buffer**: `PHY_CAL_DATA` — zeroed initially (no prior
calibration). Includes `version[4]`, `mac[6]`, `opaque[1894]`.

**Digital register backup**: `SOC_PHY_DIG_REGS_MEM` — 84 bytes
(21 × 4) for PHY register save/restore across sleep cycles.

---

## 9. Memory / Heap for Blobs

The blobs need dynamic allocation. We provide a 128 KiB first-fit
free-list allocator in `crates/soc/esp32/src/heap.rs`.

- Backing store: `static mut HEAP: [u8; 128*1024]` in BSS (DRAM)
- Block header: 8 bytes (`size: usize`, `next: *mut BlockHeader`)
- Alignment: 4 bytes (RISC-V 32-bit)
- Thread safety: relies on blob calling `wifi_int_disable` / `wifi_int_restore`
- Debug: prints 'M' + 4 hex nibbles on allocation failure
- Exports: `malloc`, `free`, `calloc`, `realloc`, `free_heap_size`

---

## 10. WiFi Init Flow

The `Esp32Wifi::init()` function in `wifi.rs` runs these steps. Each
step emits a debug character to the USB Serial JTAG FIFO (0x6000F000).

| Step | Char | Action | Blob Function |
|------|------|--------|---------------|
| 1 | `1` | Enable modem clocks + reset | `modem::enable_all_clocks()`, `modem::reset_all_modems()` |
| 2 | `2` | Read factory MAC from eFuse | `modem::read_efuse_mac()` |
| 3 | `3` | Set up OSI function table | Copy `g_wifi_osi_funcs` → blob extern, write `g_osi_funcs_p` |
| — | `@` | Verify: dump OSI table address | Debug: 8 hex nibbles |
| — | `P` | Verify: dump `g_osi_funcs_p` readback | Debug: 8 hex nibbles |
| — | `F` | Verify: dump `set_intr` fn ptr (offset 8) | Debug: 8 hex nibbles |
| — | `G` | Verify: dump `set_isr` fn ptr (offset 16) | Debug: 8 hex nibbles |
| 3b | — | Configure WiFi interrupt routing | `setup_wifi_interrupts()` |
| 4 | — | Build `wifi_init_config_t` | Static config struct |
| 5 | `5` | Initialize WiFi internals | `esp_wifi_init_internal(&init_cfg)` |
| 6 | `6` | Initialize WPA2 supplicant | `esp_supplicant_init()` |
| 7 | `7` | Set STA mode | `esp_wifi_set_mode(WIFI_MODE_STA)` |
| 8 | `8` | Configure SSID + password | `esp_wifi_set_config(STA, &cfg)` |
| 9 | `9` | Register RX callback | `esp_wifi_internal_reg_rxcb(STA, recv_cb_sta)` |
| A | `A` | Start WiFi | `esp_wifi_start()` |
| — | `<` | Dump assertion bytes at 0x408573ED/EE | Debug: `<XX,YY>` |

**Step 5 details** (`esp_wifi_init_internal`):

The `wifi_init_config_t` struct we pass:
- `static_rx_buf_num = 4`, `dynamic_rx_buf_num = 4`
- `tx_buf_type = 0` (static), `static_tx_buf_num = 4`
- `nvs_enable = 0` (no NVS)
- `ampdu_rx_enable = 0`, `ampdu_tx_enable = 0`
- `beacon_max_len = 752`
- `feature_caps = (1<<0) | (1<<7)` (WPA3-SAE + Enterprise)
- `magic = WIFI_INIT_CONFIG_MAGIC`

During step 5, the blob:
- Creates semaphores (traced as 'S' + hex address)
- Creates the ppTask via `task_create_pinned_to_core` (traced as 'T' + hex)
- Creates queues (traced as 'Q' + hex address / 'W' + hex address)
- Sets up internal state structures
- Does **NOT** call `phy_enable` at this point (no 'Y' in trace)

**Step A** (`esp_wifi_start`):
- Sends a START command to ppTask's queue
- ppTask wakes up (traced as 'r' = queue recv)
- ppTask calls `wifi_start_process` → `wifi_hw_start`
- Inside `wifi_hw_start`: two assertion checks on `*(0x408573ED)` and
  `*(0x408573EE)` — if `(byte >> mode) & 1`, infinite loop. Both are 0x00;
  for mode=3 (STA), the checks pass.
- Then the execution flows through the functions listed in §7

---

## 11. WiFi Connect Flow

After init succeeds, subsequent calls to `Esp32Wifi::init()` take the
"already initialized" path:

| Step | Char | Action |
|------|------|--------|
| B | `B` | `esp_wifi_connect_internal()` |
| C | `C` | 200 iterations of `poll_timers()` + `yield_to_scheduler()` |
| D | `D` | Set `self.connected = true` |

The connect path has never been reached because init blocks at
`esp_wifi_start`.

---

## 12. Current Blocking Point — Root Cause Analysis

### What happens

`esp_wifi_start()` returns ESP_OK, but then ppTask runs
`wifi_start_process()` → `wifi_hw_start()` → `phy_enable()` →
`register_chipv7_phy()`, which **never returns**.

### Serial output evidence

```
123@4080f8e4,P4080f8e4,F4222e30c,G4222e356,5gW4084fa00,S4085708c,T0007
t4085708c,R0gwpgt4085708c,rgggggggggggggggggggggggggwg6gpgt4085708c,rgw7
gpgt4085708c,rgw8gpgt4085708c,rgw9gpgt4085708c,rgwgA<00,00>gpgt4085708c,
rggCY12
```

**Decoded trace** (see §14 for abbreviations):
- `1` → Step 1: clocks enabled
- `2` → Step 2: MAC read
- `3` → Step 3: OSI table set up
- `@4080f8e4` → OSI table at 0x4080F8E4
- `P4080f8e4` → g_osi_funcs_p readback matches ✓
- `F4222e30c` → set_intr fn ptr at 0x4222E30C ✓
- `G4222e356` → set_isr fn ptr at 0x4222E356 ✓
- `5` → Step 5: esp_wifi_init_internal called
- `g` → semaphore given
- `W4084fa00` → wifi_create_queue returned 0x4084FA00
- `S4085708c` → semphr_create returned 0x4085708C
- `T0007` → task_create spawned child with task ID 7
- `t4085708c,R0` → ppTask blocking on semaphore, then runs (slot 0)
- Various `g`, `w`, `p`, `r` → queue/semaphore operations during init
- `6` → Step 6: supplicant init
- `7` → Step 7: STA mode
- `8` → Step 8: credentials configured
- `9` → Step 9: RX callback registered
- `A` → Step A: esp_wifi_start
- `<00,00>` → assertion bytes both 0x00 (pass for mode 3)
- After `A`: more queue/semaphore ops, then START command sent
- `r` → ppTask receives command from queue
- `gg` → two semaphore gives
- `C` → wifi_clock_enable called
- `Y` → phy_enable entered
- `1` → PHY_ENABLED was false
- `2` → PHY_CALIBRATED was false → entering register_chipv7_phy
- **SILENCE** — register_chipv7_phy never returns

### What register_chipv7_phy does (from libphy.a)

This ROM function performs full RF calibration:
1. Programs PHY registers from the 128-byte init data
2. Runs calibration routines (DC offset, IQ imbalance, etc.)
3. May require hardware completion signals / interrupts
4. Writes calibration results to cal_data buffer

### Why it blocks — hypotheses

1. **Interrupts needed**: The PHY calibration process may trigger
   hardware events that require interrupt handling to complete. But
   `ic_set_interrupt_handler` hasn't been called yet (it comes AFTER
   phy_enable in `wifi_hw_start`), so WiFi ISR isn't registered. If
   the calibration waits for an interrupt-driven acknowledgement, it
   would block forever.

2. **Modem clock/power state**: The modem may need additional clock
   domains or power rails enabled before calibration can proceed.

3. **Context issue**: ppTask runs via SYS_SPAWN with its own stack.
   If register_chipv7_phy expects to be called from a specific
   context (e.g., with certain CSR settings or privilege level),
   it may behave unexpectedly.

4. **Missing `g_phyFuns` or related**: Earlier sessions fixed a
   crash where `g_phyFuns` was NULL. The fix was to provide ROM
   symbols via `PROVIDE()` in the linker script. If any related
   symbol is still missing, the function may hang.

5. **Early PHY init needed**: esp-wifi calls PHY init early, during
   `init()` (before `esp_wifi_init_internal`), NOT from within
   ppTask via `phy_enable`. This means by the time wifi_hw_start
   calls phy_enable, `PHY_CALIBRATED` is already `true`, and it
   takes the fast `phy_wakeup_init()` path instead.

### Most likely fix

**Hypothesis 5 is the most promising**. The fix would be:

1. Call `register_chipv7_phy()` early — from the wifi-drv thread
   context during Step 3b or just before Step 5, similar to esp-wifi's
   `init()` function:
   ```rust
   // After modem clocks enabled, before esp_wifi_init_internal:
   modem::enable_all_clocks();
   register_chipv7_phy(&PHY_INIT_DATA, &mut PHY_CAL_DATA, PHY_RF_CAL_FULL);
   PHY_CALIBRATED = true;
   PHY_ENABLED = true;
   ```

2. When the blob later calls `phy_enable()` from ppTask, it would find
   `PHY_CALIBRATED == true` and take the fast `phy_wakeup_init()` path.

This matches esp-wifi's approach in `esp-wifi/src/lib.rs`:
```rust
pub fn init(...) -> ... {
    // ...
    unsafe { phy_enable_clock(); }
    unsafe {
        register_chipv7_phy(
            &PHY_INIT_DATA_DEFAULT,
            &mut cal_data,
            PHY_CALIBRATION_MODE,
        );
    }
    // ...
    esp_wifi_init_internal(&cfg);
    // ...
}
```

---

## 13. Configuration System

WiFi credentials are stored in the VeerOS virtual filesystem:

**File**: `/etc/net/wifi`  
**Format**: Key=value text (parsed by `userlib::config::Config`):
```
# VeerOS WiFi configuration
ssid=MARS
password=Naitla123
```

**Build-time configuration**:
```bash
./scripts/build-esp32c6.sh --ssid MyNetwork --password MyPass123
```

Sets environment variables `VEEROS_WIFI_SSID` and `VEEROS_WIFI_PASS`,
compiled into the binary via `option_env!()`. Defaults: SSID=`MARS`,
password=`Naitla123`.

**Runtime flow**:
1. Boot creates `/etc/net/wifi` in ramfs from build-time values
2. `wifi_driver_task()` reads `/etc/net/wifi` on start
3. Parses with `Config::parse()`, extracts `ssid` and `password`
4. Calls `mgr.set_credentials(ssid, pass)`
5. Enters connect retry loop

---

## 14. Trace Marker Reference

Debug output goes to USB Serial JTAG FIFO at `0x6000F000` (data)
and `0x6000F004` (control). All markers are single ASCII characters.

### Init flow markers (wifi.rs)

| Char | Meaning |
|------|---------|
| `1`-`9`, `A` | Init steps 1-10 |
| `@XXXXXXXX` | OSI table address (8 hex nibbles) |
| `PXXXXXXXX` | g_osi_funcs_p readback |
| `FXXXXXXXX` | set_intr fn ptr value |
| `GXXXXXXXX` | set_isr fn ptr value |
| `<XX,YY>` | Assertion bytes at 0x408573ED, 0x408573EE |
| `B` | Connect path (already initialized) |
| `C` | Connect: yield loop |
| `D` | Connect: done |
| `!N` | Error at step N |

### OSI adapter markers (wifi_os_adapter.rs)

| Char | Meaning |
|------|---------|
| `I` | `set_intr` called (+ source:cpu_int hex) |
| `J` | `set_isr` called (ISR handler stored) |
| `E` | `ints_on` called (+ 2 hex nibbles of mask) |
| `S` | `semphr_create` (+ 8 hex nibbles: handle address) |
| `t` | `semphr_take` blocking (+ 8 hex nibbles: handle address) |
| `g` | `semphr_give` |
| `Q` | `queue_create` (+ 8 hex nibbles: handle address) |
| `W` | `wifi_create_queue` (+ 8 hex nibbles: handle address) |
| `p` | `queue_send` success |
| `F` | `queue_send` failed (full) |
| `w` | `queue_recv` waiting |
| `r` | `queue_recv` received |
| `b` | `event_group_set_bits` (+ 4 hex nibbles) |
| `W` | `event_group_wait_bits` (+ 4 hex nibbles) |
| `T` | `task_create` (+ 4 hex nibbles: child task ID) |
| `R` | `run_blob_task` (+ slot index digit) |
| `X` | blob task exited (+ slot index digit) |
| `C` | `wifi_clock_enable` |
| `Y` | `phy_enable` entered |
| `1` | PHY_ENABLED was false |
| `2` | PHY_CALIBRATED was false (full cal path) |
| `3` | register_chipv7_phy returned (post-calibration) |
| `4` | PHY_CALIBRATED was true (wakeup path) |
| `5` | phy_wakeup_init returned |
| `Z` | `phy_enable` exit |
| `X` | `coex_enable` |
| `M` | `wifi_reset_mac` / `osi_malloc` failure |
| `i` | WiFi ISR dispatch (+ cpu_int hex digit) |
| `E` | `event_post` (+ 2 hex nibbles of event_id) |

Note: Some characters are overloaded (e.g., `W` for both
`wifi_create_queue` and `event_group_wait_bits`; `M` for both
`wifi_reset_mac` and malloc failure). Context disambiguates.

---

## 15. Key Addresses & Symbols

### ROM / Blob symbols (from `llvm-nm`)

| Symbol | Address | Library |
|--------|---------|---------|
| `ic_set_interrupt_handler` | `0x42261FA8` | libpp.a |
| `wifi_hw_start` | `0x4224C0E6` | libnet80211.a |
| `wifi_start_process` | `0x4224C822` | libnet80211.a |
| `ppTask` | `0x4227F7CC` | libpp.a |
| `register_chipv7_phy` | (libphy.a) | libphy.a |
| `g_osi_funcs_p` | `0x4087FF6C` | ROM data |
| `pp_task_hdl` | `0x4087FF40` | ROM data |
| `s_wifi_queue` | `0x4087FF44` | ROM data |
| `g_ic` | `0x40857298` | ROM data |

### Hardware registers

| Name | Address | Purpose |
|------|---------|---------|
| USB Serial JTAG data | `0x6000F000` | Debug output FIFO |
| USB Serial JTAG ctrl | `0x6000F004` | Flush control |
| INTMATRIX base | `0x60010000` | Peripheral → CPU int routing |
| PLIC base | `0x20001000` | Enable + priority + threshold |
| MODEM_LPCON | `0x600AF000` | Modem clock gates |
| MODEM_SYSCON | `0x600A9800` | Modem system config |
| WIFI_MAC base | `0x600A4000` | WiFi MAC registers |
| WIFI_BB base | `0x600A7800` | WiFi baseband registers |
| EFUSE base | `0x600B0800` | Factory data (MAC etc.) |
| SYSTIMER | `0x60004000` | Kernel tick timer |
| RNG | `0x600260B0` | Hardware random number generator |

### Assertion bytes (wifi_hw_start)

| Address | Meaning |
|---------|---------|
| `0x408573ED` | Byte 1: if `(byte >> mode) & 1` → infinite loop |
| `0x408573EE` | Byte 2: if `(byte >> mode) & 1` → infinite loop |

Both are 0x00 for our STA mode (mode=3), so assertions pass.

---

## 16. Build, Flash & Monitor

### Build

```bash
cargo build \
  --target riscv32imc-unknown-none-elf \
  -p kernel-xiao-esp32c6 \
  --no-default-features \
  --features "dist-minimal,shell,wifi,ble,ieee802154" \
  --release
```

Or with the build script:
```bash
./scripts/build-esp32c6.sh --ssid MARS --password Naitla123
```

### Flash

```bash
espflash flash \
  --chip esp32c6 \
  --port /dev/ttyACM0 \
  --baud 921600 \
  --partition-table crates/kernel/xiao_esp32c6/partitions.csv \
  --ignore-app-descriptor \
  target/riscv32imc-unknown-none-elf/release/kernel-xiao-esp32c6
```

### Monitor

```bash
picocom -b 115200 /dev/ttyACM0
```

Or with the build script:
```bash
./scripts/build-esp32c6.sh  # builds, flashes, and monitors
```

---

## 17. Serial Output Decoding

Example raw output and full decode:

```
123@4080f8e4,P4080f8e4,F4222e30c,G4222e356,5gW4084fa00,S4085708c,T0007
t4085708c,R0gwpgt4085708c,rgggggggggggggggggggggggggwg6gpgt4085708c,rgw7
gpgt4085708c,rgw8gpgt4085708c,rgw9gpgt4085708c,rgwgA<00,00>gpgt4085708c,
rggCY12
```

**Decoded:**

| Trace | Meaning |
|-------|---------|
| `1` | Step 1: modem clocks + reset |
| `2` | Step 2: eFuse MAC read |
| `3` | Step 3: OSI table setup |
| `@4080f8e4` | OSI table at 0x4080F8E4 |
| `,P4080f8e4` | g_osi_funcs_p confirmed pointing to table |
| `,F4222e30c` | set_intr fn ptr = 0x4222E30C (matches llvm-nm) |
| `,G4222e356` | set_isr fn ptr = 0x4222E356 (matches llvm-nm) |
| `,5` | Step 5: esp_wifi_init_internal |
| `g` | Semaphore given (during init) |
| `W4084fa00` | wifi_create_queue: handle 0x4084FA00 |
| `,S4085708c` | semphr_create: handle 0x4085708C |
| `,T0007` | task_create: ppTask is task #7 |
| `t4085708c,` | ppTask waits on semaphore 0x4085708C |
| `R0` | Blob task slot 0 (ppTask) starts running |
| `gwpg` | give, wait, put, give — ppTask initial setup |
| `t4085708c,` | ppTask blocks on semaphore again |
| `r` | queue_recv completed |
| `g...g` | Many semaphore gives during init |
| `wg6` | queue wait, give, Step 6 (supplicant) |
| `gpgt4085708c,rgw` | queue/sem ops + Step 7 |
| `7gpgt4085708c,rgw` | more ops + Step 8 |
| `8gpgt4085708c,rgw` | more ops + Step 9 |
| `9gpgt4085708c,rgwg` | more ops |
| `A` | Step A: esp_wifi_start |
| `<00,00>` | Assertion bytes: OK |
| `gpgt4085708c,` | START command queued |
| `r` | ppTask receives START |
| `gg` | Two semaphore gives |
| `C` | wifi_clock_enable |
| `Y` | phy_enable entered |
| `1` | PHY_ENABLED = false |
| `2` | PHY_CALIBRATED = false → calling register_chipv7_phy |
| (silence) | **BLOCKED** — register_chipv7_phy hangs |

---

## 18. Next Steps & Hypotheses

### Priority 1: Early PHY calibration

Call `register_chipv7_phy()` from the wifi-drv thread context BEFORE
`esp_wifi_init_internal`, matching esp-wifi's init order:

```rust
// In Esp32Wifi::init(), after Step 3b, before Step 5:
modem::enable_all_clocks();
unsafe {
    register_chipv7_phy(&PHY_INIT_DATA, &mut PHY_CAL_DATA, PHY_RF_CAL_FULL);
    PHY_CALIBRATED = true;
    PHY_ENABLED = true;
}
```

This way, the blob's `phy_enable` call from ppTask finds `PHY_CALIBRATED
== true` and takes the fast `phy_wakeup_init()` path.

### Priority 2: If early cal also blocks

If `register_chipv7_phy` also blocks in the wifi-drv thread, investigate:

1. **Add digital register backup pointer**: esp-wifi calls
   `phy_dig_reg_backup(true, &mut SOC_PHY_DIG_REGS_MEM)` before
   calibration. May be required.

2. **Check modem clock state**: Dump MODEM_LPCON and MODEM_SYSCON
   registers before calling register_chipv7_phy. Ensure WiFi + BLE +
   FE clocks are all enabled.

3. **Check interrupt state**: Does register_chipv7_phy need interrupts
   enabled? Try calling with `mstatus.MIE = 1` explicitly.

4. **Add `phy_get_romfunc_addr`**: Check if the PHY calibration needs
   ROM function addresses set up (g_phyFuns).

5. **Compare with esp-wifi register writes**: esp-wifi does additional
   setup before calling register_chipv7_phy:
   - `phy_enable_clock()` — enables PHY peripheral clock specifically
   - Copies `PHY_INIT_DATA` and may modify params
   - Calls with specific calibration mode

### Priority 3: After PHY unblocked

Once `register_chipv7_phy` succeeds, the remaining `wifi_hw_start`
sequence should proceed:
- `coex_enable()` → no-op ✓
- `wifi_reset_mac()` → `modem::reset_all_modems()` ✓
- `ic_mac_init()` → internal setup
- `chm_init()` → channel management
- `ic_set_interrupt_handler()` → will store our ISR handler ✓
- Event bits get set → unblocks `esp_wifi_start` caller ✓

Then test:
1. `esp_wifi_start` returns to wifi_driver_task
2. `esp_wifi_connect_internal` triggers WPA2 handshake
3. ISR fires (verify with 'i2' or 'i3' traces)
4. Connection established
5. TX/RX over the air

---

## 19. Reference: esp-wifi Comparison

The esp-wifi v0.15.1 crate (for `no_std` Rust on ESP-IDF-less targets)
provides a reference implementation. Key differences from our approach:

| Aspect | esp-wifi | VeerOS |
|--------|----------|--------|
| Task model | Single-threaded with `yield_task()` coroutines | Real preemptive scheduler tasks via SYS_SPAWN |
| PHY init | Early, in `init()` before `esp_wifi_init_internal` | Late, in ppTask via `phy_enable()` — **THIS IS THE BUG** |
| ISR setup | `set_isr` stores handler; `WIFI_MAC()` / `WIFI_PWR()` are `#[no_mangle]` entry points | `set_isr` stores handler; `wifi_isr_dispatch()` from trap handler |
| `set_intr` | No-op | No-op (traces only) |
| Queue | Static arrays, `critical_section` for sync | Ring buffer `SimpleQueue` with interrupt-disable |
| Semaphore | CriticalSection-based binary sem | `SimpleSem` with spin-wait + SYS_SLEEP |
| Timer | Embassy-based or poll loop | Software timer with `poll_timers()` |
| Heap | `esp-alloc` or `embedded-alloc` | Custom first-fit 128 KiB |

### esp-wifi init order (reference)

```rust
fn init() {
    // 1. Enable modem clocks
    modem_clock_select_lp_clock_source();
    reset_mac();

    // 2. PHY calibration (EARLY!)
    phy_enable_clock();
    register_chipv7_phy(&init_data, &mut cal_data, cal_mode);

    // 3. Set OSI funcs
    g_wifi_osi_funcs = &WIFI_OS_FUNCS;

    // 4. esp_wifi_init_internal(&cfg)
    // 5. esp_wifi_set_mode
    // 6. esp_wifi_set_config
    // 7. esp_wifi_start
    // 8. esp_wifi_connect (if auto-connect)
}
```

---

## 20. Appendix: OSI Function Table Layout

The `wifi_osi_funcs_t` struct fields in order (field index and byte offset):

| # | Offset | Field | Our Function |
|---|--------|-------|--------------|
| 0 | 0 | `_version` | `ESP_WIFI_OS_ADAPTER_VERSION` |
| 1 | 4 | `_env_is_chip` | `env_is_chip` → true |
| 2 | 8 | `_set_intr` | `set_intr` |
| 3 | 12 | `_clear_intr` | `clear_intr` |
| 4 | 16 | `_set_isr` | `set_isr` |
| 5 | 20 | `_ints_on` | `ints_on` |
| 6 | 24 | `_ints_off` | `ints_off` |
| 7 | 28 | `_is_from_isr` | `is_from_isr` → false |
| 8 | 32 | `_spin_lock_create` | `spin_lock_create` → dummy |
| 9 | 36 | `_spin_lock_delete` | `spin_lock_delete` → no-op |
| 10 | 40 | `_wifi_int_disable` | `wifi_int_disable` → csrrci mstatus |
| 11 | 44 | `_wifi_int_restore` | `wifi_int_restore` |
| 12 | 48 | `_task_yield_from_isr` | no-op |
| 13 | 52 | `_semphr_create` | `semphr_create` |
| 14 | 56 | `_semphr_delete` | `semphr_delete` |
| 15 | 60 | `_semphr_take` | `semphr_take` |
| 16 | 64 | `_semphr_give` | `semphr_give` |
| 17 | 68 | `_wifi_thread_semphr_get` | `wifi_thread_semphr_get` |
| 18 | 72 | `_mutex_create` | `mutex_create` |
| 19 | 76 | `_recursive_mutex_create` | `recursive_mutex_create` |
| 20 | 80 | `_mutex_delete` | `mutex_delete` |
| 21 | 84 | `_mutex_lock` | `mutex_lock` |
| 22 | 88 | `_mutex_unlock` | `mutex_unlock` |
| 23 | 92 | `_queue_create` | `queue_create` |
| 24 | 96 | `_queue_delete` | `queue_delete` |
| 25 | 100 | `_queue_send` | `queue_send` |
| 26 | 104 | `_queue_send_from_isr` | `queue_send_from_isr` |
| 27 | 108 | `_queue_send_to_back` | `queue_send_to_back` |
| 28 | 112 | `_queue_send_to_front` | `queue_send_to_front` |
| 29 | 116 | `_queue_recv` | `queue_recv` |
| 30 | 120 | `_queue_msg_waiting` | `queue_msg_waiting` |
| 31 | 124 | `_event_group_create` | `event_group_create` |
| 32 | 128 | `_event_group_delete` | `event_group_delete` |
| 33 | 132 | `_event_group_set_bits` | `event_group_set_bits` |
| 34 | 136 | `_event_group_clear_bits` | `event_group_clear_bits` |
| 35 | 140 | `_event_group_wait_bits` | `event_group_wait_bits` |
| 36 | 144 | `_task_create_pinned_to_core` | `task_create_pinned_to_core` |
| 37 | 148 | `_task_create` | `task_create` |
| 38 | 152 | `_task_delete` | `task_delete` → no-op |
| 39 | 156 | `_task_delay` | `task_delay` → SYS_SLEEP |
| 40 | 160 | `_task_ms_to_tick` | `task_ms_to_tick` (1 tick = 10ms) |
| 41 | 164 | `_task_get_current_task` | → SYS_TASK_ID |
| 42 | 168 | `_task_get_max_priority` | → 25 |
| 43 | 172 | `_malloc` | `osi_malloc` |
| 44 | 176 | `_free` | `osi_free` |
| 45 | 180 | `_event_post` | `event_post` (traces E+id) |
| 46 | 184 | `_get_free_heap_size` | `get_free_heap_size` |
| 47 | 188 | `_rand` | `rand` → RNG register |
| 48 | 192 | `_dport_access_stall_other_cpu_start` | no-op |
| 49 | 196 | `_dport_access_stall_other_cpu_end` | no-op |
| 50 | 200 | `_wifi_apb80m_request` | no-op |
| 51 | 204 | `_wifi_apb80m_release` | no-op |
| 52 | 208 | `_phy_disable` | `phy_disable` |
| 53 | 212 | `_phy_enable` | `phy_enable` |
| 54 | 216 | `_phy_update_country_info` | → 0 |
| 55 | 220 | `_read_mac` | `read_mac` → eFuse |
| 56 | 224 | `_timer_arm` | `ets_timer_arm` |
| 57 | 228 | `_timer_disarm` | `ets_timer_disarm` |
| 58 | 232 | `_timer_done` | `ets_timer_done` |
| 59 | 236 | `_timer_setfn` | `ets_timer_setfn` |
| 60 | 240 | `_timer_arm_us` | `ets_timer_arm_us` |
| 61 | 244 | `_wifi_reset_mac` | `wifi_reset_mac` |
| 62 | 248 | `_wifi_clock_enable` | `wifi_clock_enable` |
| 63 | 252 | `_wifi_clock_disable` | no-op |
| 64 | 256 | `_wifi_rtc_enable_iso` | no-op |
| 65 | 260 | `_wifi_rtc_disable_iso` | no-op |
| 66 | 264 | `_esp_timer_get_time` | `systimer::now_us()` |
| 67-79 | 268-332 | NVS functions | All return -1 |
| 80 | 336 | `_get_random` | `get_random` |
| 81 | 340 | `_get_time` | `get_time` → systimer |
| 82 | 344 | `_random` | `random` → RNG |
| 83 | 348 | `_slowclk_cal_get` | → 6667 |
| 84 | 352 | `_log_write` | None |
| 85 | 356 | `_log_writev` | None |
| 86 | 360 | `_log_timestamp` | `log_timestamp` |
| 87-94 | 364-392 | Internal alloc + wifi alloc | Various malloc/calloc wrappers |
| 95-96 | 396-400 | wifi_create/delete_queue | Custom queue alloc |
| 97-116 | 404-480 | Coex functions | All no-ops returning 0 |
| 117 | 484 | `_magic` | `ESP_WIFI_OS_ADAPTER_MAGIC` |

---

*Last updated: current session. Document reflects state as of the
`register_chipv7_phy` blocking investigation.*
