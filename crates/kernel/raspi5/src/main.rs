//! VeerOS kernel for Raspberry Pi 5 (BCM2712, AArch64).
//!
//! Bare-metal kernel: `#![no_std]`, `#![no_main]`, runs on AArch64 at EL1
//! with ARM generic timer interrupts and the GIC-400 interrupt controller.
//!
//! Build:
//!   cargo build -p kernel-raspi5 --target aarch64-unknown-none-softfloat
//!
//! Deploy:
//!   Copy `target/aarch64-unknown-none-softfloat/debug/kernel-raspi5`
//!   to the SD card as `kernel8.img`.  Ensure `config.txt` contains
//!   `arm_64bit=1` and `enable_uart=1`.

#![no_std]
#![no_main]

mod trap;
#[cfg(feature = "samples")]
mod samples;

use core::cell::UnsafeCell;
use core::fmt::Write;

use arch::{Console, SavedContext, TickTimer, Platform};
use soc_raspi5::{system_timer, Raspi5};
use soc_raspi5::gic::Gic400;
use soc_raspi5::fbcon::FbConsole;
use microkernel::Kernel;
use microkernel::alloc::Heap;
use microkernel::channel::Channels;
use microkernel::driver::{DriverCaps, DriverRegistry, MemRegion};
use microkernel::futex::FutexTable;
use microkernel::ipc::Ipc;
use microkernel::process::ProcessTable;
use microkernel::socket::SocketTable;
use microkernel::task::Scheduler;
#[cfg(feature = "shell")]
use microkernel::task::TaskState;
#[cfg(feature = "shell")]
use shell::{Shell, ShellEnv};
use microkernel::user::UserTable;

use panic_halt as _;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const TICK_PERIOD_US: u32 = 1_000; // 1 ms
// ═══════════════════════════════════════════════════════════════════════════
// Static kernel state (single-core, interrupts disabled during access)
// ═══════════════════════════════════════════════════════════════════════════

struct SchedulerCell(UnsafeCell<Scheduler>);
unsafe impl Sync for SchedulerCell {}
pub(crate) static SCHEDULER: SchedulerCell = SchedulerCell(UnsafeCell::new(Scheduler::new()));

// Kernel heap (64 KiB)
const HEAP_SIZE: usize = 64 * 1024;
const HEAP_SMALL_BYTES: usize = 16 * 1024;

#[repr(align(16))]
struct HeapRegion([u8; HEAP_SIZE]);
static mut HEAP_REGION: HeapRegion = HeapRegion([0u8; HEAP_SIZE]);

struct HeapCell(UnsafeCell<Heap>);
unsafe impl Sync for HeapCell {}
pub(crate) static HEAP: HeapCell = HeapCell(UnsafeCell::new(Heap::new()));

// IPC mailboxes
struct IpcCell(UnsafeCell<Ipc>);
unsafe impl Sync for IpcCell {}
pub(crate) static IPC: IpcCell = IpcCell(UnsafeCell::new(Ipc::new()));

// Futex table
struct FutexCell(UnsafeCell<FutexTable>);
unsafe impl Sync for FutexCell {}
pub(crate) static FUTEX: FutexCell = FutexCell(UnsafeCell::new(FutexTable::new()));

// Channels
struct ChannelCell(UnsafeCell<Channels>);
unsafe impl Sync for ChannelCell {}
pub(crate) static CHANNELS: ChannelCell = ChannelCell(UnsafeCell::new(Channels::new()));

// Poll table
use microkernel::poll::PollTable;
struct PollCell(UnsafeCell<PollTable>);
unsafe impl Sync for PollCell {}
pub(crate) static POLL: PollCell = PollCell(UnsafeCell::new(PollTable::new()));

// Process table
pub(crate) struct ProcessCell(pub UnsafeCell<ProcessTable>);
unsafe impl Sync for ProcessCell {}
pub(crate) static PROCESSES: ProcessCell = ProcessCell(UnsafeCell::new(ProcessTable::new()));

// Socket table
pub(crate) struct SocketCell(pub UnsafeCell<SocketTable>);
unsafe impl Sync for SocketCell {}
pub(crate) static SOCKETS: SocketCell = SocketCell(UnsafeCell::new(SocketTable::new()));

// User table
pub(crate) struct UserCell(pub UnsafeCell<UserTable>);
unsafe impl Sync for UserCell {}
pub(crate) static USERS: UserCell = UserCell(UnsafeCell::new(UserTable::new()));

// VFS inode table
use microkernel::vfs::InodeTable;
pub(crate) struct InodeCell(pub UnsafeCell<InodeTable>);
unsafe impl Sync for InodeCell {}
pub(crate) static INODES: InodeCell = InodeCell(UnsafeCell::new(InodeTable::new()));

// RamFS
use microkernel::ramfs::RamFs;
pub(crate) struct RamFsCell(pub UnsafeCell<RamFs>);
unsafe impl Sync for RamFsCell {}
pub(crate) static RAMFS: RamFsCell = RamFsCell(UnsafeCell::new(RamFs::new()));

// FAT32 filesystem
use microkernel::fat32::Fat32;
pub(crate) struct Fat32Cell(pub UnsafeCell<Fat32>);
unsafe impl Sync for Fat32Cell {}
pub(crate) static FAT32: Fat32Cell = Fat32Cell(UnsafeCell::new(Fat32::new()));

// Mount table
use microkernel::vfs::MountTable;
pub(crate) struct MountCell(pub UnsafeCell<MountTable>);
unsafe impl Sync for MountCell {}
pub(crate) static MOUNTS: MountCell = MountCell(UnsafeCell::new(MountTable::new()));

// ---------------------------------------------------------------------------
// Input subsystem
// ---------------------------------------------------------------------------

use microkernel::input::InputSubsystem;

pub(crate) struct InputCell(pub UnsafeCell<InputSubsystem>);
unsafe impl Sync for InputCell {}
pub(crate) static INPUT: InputCell = InputCell(UnsafeCell::new(InputSubsystem::new()));

// xHCI USB host controllers
use soc_raspi5::xhci::Xhci;
struct XhciCell(UnsafeCell<Xhci>);
unsafe impl Sync for XhciCell {}
static XHCI0: XhciCell = XhciCell(UnsafeCell::new(Xhci::xhci0()));
static XHCI1: XhciCell = XhciCell(UnsafeCell::new(Xhci::xhci1()));

// SD card (EMMC2)
use soc_raspi5::sd::Emmc2Sd;
struct SdCell(UnsafeCell<Emmc2Sd>);
unsafe impl Sync for SdCell {}
static SD: SdCell = SdCell(UnsafeCell::new(Emmc2Sd::new()));

// Driver registry
struct RegistryCell(UnsafeCell<DriverRegistry>);
unsafe impl Sync for RegistryCell {}
static DRIVERS: RegistryCell = RegistryCell(UnsafeCell::new(DriverRegistry::new()));

// Audit log
use microkernel::audit::AuditLog;
pub(crate) struct AuditCell(pub UnsafeCell<AuditLog>);
unsafe impl Sync for AuditCell {}
pub(crate) static AUDIT: AuditCell = AuditCell(UnsafeCell::new(AuditLog::new()));

// AI-Native Execution subsystems
use microkernel::agent::AgentTable;
use microkernel::intent::IntentEngine;
use microkernel::memory_engine::MemoryEngine;
use microkernel::fabric::ExecutionFabric;
use microkernel::intent_sched::IntentScheduler;

pub(crate) struct AgentCell(pub UnsafeCell<AgentTable>);
unsafe impl Sync for AgentCell {}
pub(crate) static AGENTS: AgentCell = AgentCell(UnsafeCell::new(AgentTable::new()));

pub(crate) struct IntentCell(pub UnsafeCell<IntentEngine>);
unsafe impl Sync for IntentCell {}
pub(crate) static INTENTS: IntentCell = IntentCell(UnsafeCell::new(IntentEngine::new()));

pub(crate) struct MemoryEngineCell(pub UnsafeCell<MemoryEngine>);
unsafe impl Sync for MemoryEngineCell {}
pub(crate) static MEMORY_ENGINE: MemoryEngineCell = MemoryEngineCell(UnsafeCell::new(MemoryEngine::new()));

pub(crate) struct FabricCell(pub UnsafeCell<ExecutionFabric>);
unsafe impl Sync for FabricCell {}
pub(crate) static FABRIC: FabricCell = FabricCell(UnsafeCell::new(ExecutionFabric::new()));

pub(crate) struct IntentSchedCell(pub UnsafeCell<IntentScheduler>);
unsafe impl Sync for IntentSchedCell {}
pub(crate) static INTENT_SCHED: IntentSchedCell = IntentSchedCell(UnsafeCell::new(IntentScheduler::new()));

// Timer (stored for trap handler access)
struct TimerCell(UnsafeCell<soc_raspi5::timer::ArmGenericTimer>);
unsafe impl Sync for TimerCell {}
pub(crate) static TIMER: TimerCell = TimerCell(UnsafeCell::new(soc_raspi5::timer::ArmGenericTimer::new()));

// GIC (stored for trap handler access)
struct GicCell(UnsafeCell<Gic400>);
unsafe impl Sync for GicCell {}
pub(crate) static GIC: GicCell = GicCell(UnsafeCell::new(Gic400::new()));

// Framebuffer console (HDMI + UART dual output)
struct FbConCell(UnsafeCell<FbConsole>);
unsafe impl Sync for FbConCell {}
static FBCON: FbConCell = FbConCell(UnsafeCell::new(FbConsole::inactive()));

// Kernel log ring buffer
use microkernel::klog::KernelLog;
struct KlogCell(UnsafeCell<KernelLog>);
unsafe impl Sync for KlogCell {}
static KLOG: KlogCell = KlogCell(UnsafeCell::new(KernelLog::new()));

// ═══════════════════════════════════════════════════════════════════════════
// Task stacks
// ═══════════════════════════════════════════════════════════════════════════

#[repr(align(16))]
struct IdleStack([u8; 1024]);
static IDLE_STACK: IdleStack = IdleStack([0u8; 1024]);

#[cfg(feature = "shell")]
#[repr(align(16))]
struct ShellStack([u8; 16384]);
#[cfg(feature = "shell")]
static SHELL_STACK: ShellStack = ShellStack([0u8; 16384]);

#[cfg(feature = "samples")]
#[repr(align(16))]
struct SampleStack([u8; 8192]);
#[cfg(feature = "samples")]
static HELLO_STACK: SampleStack = SampleStack([0u8; 8192]);
#[cfg(feature = "samples")]
static TIMER_STACK: SampleStack = SampleStack([0u8; 8192]);
#[cfg(feature = "samples")]
static IPC_TX_STACK: SampleStack = SampleStack([0u8; 8192]);
#[cfg(feature = "samples")]
static IPC_RX_STACK: SampleStack = SampleStack([0u8; 8192]);

// ═══════════════════════════════════════════════════════════════════════════
// Idle task
// ═══════════════════════════════════════════════════════════════════════════

fn idle_task() -> ! {
    loop {
        #[cfg(target_arch = "aarch64")]
        unsafe {
            core::arch::asm!("wfe", options(nomem, nostack));
        }
        #[cfg(not(target_arch = "aarch64"))]
        core::hint::spin_loop();
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Shell task
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(feature = "shell")]
fn shell_task() -> ! {
    let fbcon = unsafe { *FBCON.0.get() };
    let mut con = Console::new(fbcon);
    let env = ShellEnv {
        version: VERSION,
        platform: "Raspberry Pi 5 (AArch64)",
        scheduler: "minimal",
        get_uptime_ticks: Some(get_uptime_ticks),
        get_task_list: Some(write_task_list),
        get_mem_info: Some(write_mem_info),
        get_driver_list: Some(write_driver_list),
        wifi_cmd: None,
        bt_cmd: None,
        zigbee_cmd: None,
        get_current_user: Some(get_current_user),
        get_user_list: Some(write_user_list),
        vfs_list_dir: Some(vfs_list_dir),
        vfs_read_file: Some(vfs_read_file),
        vfs_write_file: Some(vfs_write_file),
        vfs_mkdir: Some(vfs_mkdir),
        vfs_stat: Some(vfs_stat),
        vfs_unlink: Some(vfs_unlink),
        vfs_rename: Some(vfs_rename),
        vfs_getcwd: Some(vfs_getcwd),
        vfs_chdir: Some(vfs_chdir),
        vfs_tree: Some(vfs_tree),
        vfs_touch: Some(vfs_touch),
        mount_list: Some(mount_list),
        mount_fs: None,
        umount_fs: None,
        lsblk: Some(lsblk_info),
        input_status: Some(input_status),
        usb_list: Some(usb_list),
        ble_hid_list: None,
        gpio_cmd: Some(gpio_cmd),
        i2c_cmd: Some(i2c_cmd),
        spi_cmd: Some(spi_cmd),
        hw_info: Some(hw_info),
        get_temp_millic: Some(get_temp_millic),
        dmesg: Some(dmesg_info),
        reboot: Some(do_reboot),
        shutdown: Some(do_shutdown),
        caps_cmd: None,
        auditlog_cmd: None,
        ifconfig_cmd: None,
        ping_cmd: None,
        netstat_cmd: None,
        ssh_cmd: None,
        #[cfg(feature = "multi-user")]
        login: Some(do_login),
        #[cfg(not(feature = "multi-user"))]
        login: None,
        #[cfg(feature = "multi-user")]
        logout: Some(do_logout),
        #[cfg(not(feature = "multi-user"))]
        logout: None,
        #[cfg(feature = "multi-user")]
        change_password: Some(do_change_password),
        #[cfg(not(feature = "multi-user"))]
        change_password: None,
        #[cfg(feature = "multi-user")]
        add_user: Some(do_add_user),
        #[cfg(not(feature = "multi-user"))]
        add_user: None,
        #[cfg(feature = "multi-user")]
        remove_user: Some(do_remove_user),
        #[cfg(not(feature = "multi-user"))]
        remove_user: None,
        pre_authenticated: false,
        // AI-native
        get_agent_list: None,
        agent_cmd: None,
        get_intent_list: None,
        intent_cmd: None,
        memory_cmd: None,
        get_fabric_status: None,
        peers_cmd: None,
        mesh_cmd: None,
        zkp_cmd: None,
    };
    let mut sh = Shell::new(env);
    loop {
        sh.run(&mut con);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// AArch64 initial SPSR_EL1 for new tasks
// ═══════════════════════════════════════════════════════════════════════════

/// SPSR_EL1 for a new task: EL1h (0x5), all interrupts unmasked.
/// When `eret` restores this, the task runs at EL1 with IRQs enabled.
const INITIAL_SPSR: usize = 0x005;

// ═══════════════════════════════════════════════════════════════════════════
// AArch64 boot assembly — EL2 → EL1, BSS zero, jump to Rust
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(
    r#"
.section .text._start
.global  _start
.balign  4

_start:
    // ── Save DTB pointer before clobbering x0 ──────────────
    mov   x19, x0

    // ── Park secondary cores (only core 0 boots) ────────────
    mrs   x0, mpidr_el1
    and   x0, x0, #0xFF
    cbnz  x0, _park

    // ── Check current exception level ────────────────────────
    mrs   x0, CurrentEL
    lsr   x0, x0, #2
    cmp   x0, #2
    b.ne  _at_el1          // Already at EL1 — skip drop

    // ── EL2 → EL1 transition ────────────────────────────────
    // Configure HCR_EL2: set RW (bit 31) so EL1 uses AArch64.
    mov   x0, #(1 << 31)
    msr   hcr_el2, x0

    // Allow EL1 access to physical timer registers.
    // CNTHCTL_EL2: EL1PCTEN (bit 0) + EL1PCEN (bit 1) = 0x3
    mov   x0, #3
    msr   cnthctl_el2, x0

    // No virtual offset.
    msr   cntvoff_el2, xzr

    // Set SCTLR_EL1 to a known safe state (caches/MMU off).
    mov   x0, xzr
    msr   sctlr_el1, x0

    // Return address and saved state for EL1.
    adr   x0, _at_el1
    msr   elr_el2, x0
    mov   x0, #0x3c5        // EL1h, DAIF all masked
    msr   spsr_el2, x0

    eret

_at_el1:
    // ── Set up the kernel stack ──────────────────────────────
    ldr   x0, =__stack_top
    mov   sp, x0

    // ── Zero BSS ─────────────────────────────────────────────
    ldr   x0, =__bss_start
    ldr   x1, =__bss_end
1:  cmp   x0, x1
    b.ge  2f
    str   xzr, [x0], #8
    b     1b
2:

    // ── Install exception vector table ───────────────────────
    ldr   x0, =_veer_vectors
    msr   vbar_el1, x0

    // ── Store DTB pointer (saved in x19 at entry, after BSS zeroed) ──
    ldr   x0, =DTB_PTR
    str   x19, [x0]

    // ── Jump into Rust ───────────────────────────────────────
    bl    _rust_start

    // Should never return.
    b     _park

_park:
    wfe
    b     _park
"#
);

// ═══════════════════════════════════════════════════════════════════════════
// DTB pointer saved by assembly entry
// ═══════════════════════════════════════════════════════════════════════════
#[unsafe(no_mangle)]
static mut DTB_PTR: usize = 0;

// ═══════════════════════════════════════════════════════════════════════════
// Rust entry point
// ═══════════════════════════════════════════════════════════════════════════

#[unsafe(no_mangle)]
pub extern "C" fn _rust_start() -> ! {
    // ── Set TPIDR_EL1 to a scratch context buffer ────────────
    // This is needed so that any synchronous exception (e.g. data abort
    // during the RP1 probe) has a valid context save area. Without this,
    // the trap handler would write registers to address 0.
    #[cfg(target_arch = "aarch64")]
    {
        static mut BOOT_CTX: arch::TaskContext = arch::TaskContext::zero();
        unsafe {
            let ptr = core::ptr::addr_of_mut!(BOOT_CTX) as usize;
            core::arch::asm!("msr tpidr_el1, {}", in(reg) ptr, options(nomem, nostack));
        }
    }

    // ── HDMI framebuffer (early) ─────────────────────────────
    // Try framebuffer FIRST — mailbox uses BCM2712 registers (not RP1),
    // so this works even if RP1 PCIe BAR is not mapped.  This gives us
    // HDMI output for diagnostics before touching any RP1 peripherals.
    {
        use soc_raspi5::fb;
        if let Some(info) = fb::init_framebuffer(1920, 1080, 32) {
            let fbcon = FbConsole::new(info);
            fbcon.clear_screen();
            unsafe { *FBCON.0.get() = fbcon; }
        }
    }

    // ── BCM2712 mini UART (RP1-independent serial) ─────────
    // This UART is directly on the SoC, not behind RP1 PCIe.
    // Requires `enable_uart=1` in config.txt on the SD card.
    if soc_raspi5::mini_uart::is_available() {
        soc_raspi5::mini_uart::write_str("\r\n[mini-uart] VeerOS boot — mini UART active\r\n");
    }

    // ── early console ────────────────────────────────────────
    // UART is NOT enabled yet — RP1 southbridge might not be accessible.
    // Console output goes to framebuffer only until we enable UART later.
    let fbcon = unsafe { *FBCON.0.get() };
    let mut con = Console::new(fbcon);

    // ── boot logo + banner (HDMI only at this point) ────────
    // Draw logo at top-left with a small margin, then print banner
    // text to the right of the logo using set_cursor (not spaces,
    // which would overwrite logo pixels with black).
    {
        let logo_x = 8u32; // 8px left margin
        let logo_y = 8u32; // 8px top margin
        let logo_w = fbcon.blit_logo(logo_x, logo_y);

        // Calculate text column to start after logo (+ 2 col gap)
        let text_col = if logo_w > 0 {
            (logo_x + logo_w) / soc_raspi5::font::CHAR_W + 2
        } else {
            0
        };

        // Logo is 48px tall = 3 rows at CHAR_H=16. Rows 0-2 are
        // within the logo zone; use set_cursor to skip past it.
        unsafe { soc_raspi5::fbcon::set_cursor(text_col, 0); }
        let _ = writeln!(con, "========================================");
        unsafe { soc_raspi5::fbcon::set_cursor(text_col, 1); }
        let _ = writeln!(con, "  VeerOS v{VERSION}");
        unsafe { soc_raspi5::fbcon::set_cursor(text_col, 2); }
        let _ = writeln!(con, "  Platform : Raspberry Pi 5 (BCM2712)");
        // Row 3: bottom edge of logo, still safe beside it
        unsafe { soc_raspi5::fbcon::set_cursor(text_col, 3); }
        let _ = writeln!(con, "  Arch     : AArch64 (Cortex-A76)");
        unsafe { soc_raspi5::fbcon::set_cursor(text_col, 4); }
        let _ = writeln!(con, "========================================");
        // Move cursor to full-width area below the header
        unsafe { soc_raspi5::fbcon::set_cursor(0, 5); }
        let _ = writeln!(con, "");
    }

    // ── platform init (step by step for diagnostics) ───────
    let _ = writeln!(con, "[boot] init CPU (FP/NEON)...");

    // Read current EL for diagnostics
    #[cfg(target_arch = "aarch64")]
    {
        let el: u64;
        unsafe { core::arch::asm!("mrs {}, CurrentEL", out(reg) el, options(nomem, nostack)); }
        let _ = writeln!(con, "[boot] CurrentEL = {}", (el >> 2) & 3);
    }

    let platform = Raspi5::new();
    platform.init_cpu();
    let _ = writeln!(con, "[boot] CPU OK");

    // Skip GIC + Timer for now — just confirm we can reach this point
    let kernel = Kernel::new(platform);
    let _ = writeln!(con, "[boot] Kernel struct ready");
    let _ = writeln!(con, "[boot] Scheduler: {}", kernel.scheduler_label());

    // ── UART: DO NOT ENABLE YET ────────────────────────────
    // RP1 BAR address needs verification. Keep HDMI-only for now.
    // soc_raspi5::fbcon::enable_uart_mirror();
    let _ = writeln!(con, "[boot] UART: skipped (HDMI-only mode)");

    // ── kernel heap ──────────────────────────────────────────
    unsafe {
        let region = &mut *core::ptr::addr_of_mut!(HEAP_REGION);
        (*HEAP.0.get()).init(&mut region.0, HEAP_SMALL_BYTES);
    }
    let _ = writeln!(
        con,
        "[boot] heap initialised ({} KiB — small {}K + large {}K)",
        HEAP_SIZE / 1024,
        HEAP_SMALL_BYTES / 1024,
        (HEAP_SIZE - HEAP_SMALL_BYTES) / 1024,
    );

    // ── register drivers ─────────────────────────────────────
    unsafe {
        let reg = &mut *DRIVERS.0.get();

        // PL011 UART (via RP1)
        let _uart = reg.register("uart0", DriverCaps {
            mmio_regions: 1,
            uses_interrupts: true,
            uses_dma: false,
            uses_network: false,
        }).unwrap();
        reg.grant_mmio(
            _uart,
            MemRegion::new(
                soc_raspi5::mem::RP1_BAR_BASE + soc_raspi5::mem::RP1_UART0_OFF,
                0x1000,
            ),
        )
        .ok();

        // ARM Generic Timer (no MMIO — system registers)
        let _timer = reg.register("arm-timer", DriverCaps {
            mmio_regions: 0,
            uses_interrupts: true,
            uses_dma: false,
            uses_network: false,
        }).unwrap();

        // GIC-400
        let _gic = reg.register("gic-400", DriverCaps {
            mmio_regions: 2,
            uses_interrupts: false,
            uses_dma: false,
            uses_network: false,
        }).unwrap();
        reg.grant_mmio(
            _gic,
            MemRegion::new(soc_raspi5::mem::GIC_DIST_BASE, 0x1000),
        )
        .ok();
    }
    let _ = writeln!(con, "[boot] driver registry: 3 drivers registered");

    // ── GIC initialisation ───────────────────────────────────
    let gic = Gic400::new();
    gic.init();
    unsafe { *GIC.0.get() = gic; }
    let _ = writeln!(con, "[boot] GIC-400 initialised (PPI #30 enabled)");

    // ── ARM generic timer ────────────────────────────────────
    let _ = writeln!(con, "[boot] configuring ARM timer...");
    let timer = system_timer();
    let freq = timer.frequency();
    let _ = writeln!(con, "[boot] timer freq = {} Hz", freq);
    timer.configure_tick(TICK_PERIOD_US);
    unsafe { *TIMER.0.get() = timer; }
    let _ = writeln!(con, "[boot] ARM timer tick @ {} us", TICK_PERIOD_US);

    // NOTE: IRQ is enabled later, just before eret into the first task.
    // If enabled here, timer IRQs would fire with TPIDR_EL1 unset,
    // causing the IRQ handler to save context via a wild pointer.
    let _ = writeln!(con, "[boot] IRQ will be enabled at scheduler start");

    // ── SD card (EMMC2) ──────────────────────────────────────
    let _ = writeln!(con, "[boot] SD card (EMMC2)...");
    {
        let sd = unsafe { &mut *SD.0.get() };
        if sd.init_reuse() {
            use arch::BlockDevice;
            let _ = writeln!(con, "[boot]   reuse OK, {} sectors", sd.block_count());
        } else if sd.init() {
            use arch::BlockDevice;
            let _ = writeln!(con, "[boot]   full init OK, {} sectors", sd.block_count());
        } else {
            let _ = writeln!(con, "[boot]   SD init FAILED");
        }
    }

    // ── RP1 discovery via BRCM PCIe driver ───────────────────
    let rp1_ok = {
        let _ = writeln!(con, "[boot] PCIe → RP1...");
        let rp1_info_inner = soc_raspi5::pcie::init_rp1(|args| {
            let _ = writeln!(con, "[pcie] {}", args);
        }, |addr| safe_read32(addr));
        if let Some(ref info) = rp1_info_inner {
            let _ = writeln!(con, "[boot] RP1 OK on {} — BAR {:#x} ({} KiB)",
                info.controller, info.bar_base, info.bar_size / 1024);
        } else {
            let _ = writeln!(con, "[boot] RP1 NOT FOUND");
        }
        rp1_info_inner.is_some()
    };

    // ── USB (xHCI via RP1) ───────────────────────────────────
    if rp1_ok {
        use arch::UsbHostController;
        let _ = writeln!(con, "[boot] starting xHCI0 init...");
        let xhci0 = unsafe { &mut *XHCI0.0.get() };
        if xhci0.init() {
            let _ = writeln!(con, "[boot] xHCI0 (USB 3.0): {} ports, {} devices",
                xhci0.port_count(), xhci0.num_devices);
            let kbd0 = xhci0.hid_keyboard_count();
            if kbd0 > 0 {
                let _ = writeln!(con, "[boot]   {} HID keyboard(s) on xHCI0", kbd0);
            }
        } else {
            let _ = writeln!(con, "[boot] xHCI0 (USB 3.0): init failed");
        }
        let _ = writeln!(con, "[boot] starting xHCI1 init...");
        let xhci1 = unsafe { &mut *XHCI1.0.get() };
        if xhci1.init() {
            let _ = writeln!(con, "[boot] xHCI1 (USB 2.0): {} ports, {} devices",
                xhci1.port_count(), xhci1.num_devices);
            let kbd1 = xhci1.hid_keyboard_count();
            if kbd1 > 0 {
                let _ = writeln!(con, "[boot]   {} HID keyboard(s) on xHCI1", kbd1);
            }
        } else {
            let _ = writeln!(con, "[boot] xHCI1 (USB 2.0): init failed");
        }

        // Enable UART mirror now that RP1 is confirmed accessible
        soc_raspi5::fbcon::enable_uart_mirror();
        let _ = writeln!(con, "[boot] UART0 enabled (RP1 accessible)");

        // Initialise input subsystem (USB HID keyboard/mouse)
        unsafe { (*INPUT.0.get()).init(); }
        // Register keyboard-poll callback so FbConsole can read USB keys.
        soc_raspi5::fbcon::set_kbd_poll(kbd_poll_fn, kbd_has_data_fn);
        let _ = writeln!(con, "[boot] input subsystem: active");
    } else {
        let _ = writeln!(con, "[boot] xHCI: skipped (RP1 not responding)");
        let _ = writeln!(con, "[boot] UART: skipped (RP1 not responding)");
    }

    // ── HDMI framebuffer driver registration ───────────────────
    {
        let fb_active = unsafe { (*FBCON.0.get()).is_active() };
        if fb_active {
            // Register framebuffer driver (FB was init'd early)
            unsafe {
                let reg = &mut *DRIVERS.0.get();
                let _ = reg.register("hdmi-fb", DriverCaps {
                    mmio_regions: 1,
                    uses_interrupts: false,
                    uses_dma: false,
                    uses_network: false,
                });
            }
            let _ = writeln!(con, "[boot] HDMI framebuffer: active");
        } else {
            let _ = writeln!(con, "[boot] HDMI framebuffer: not available (UART-only mode)");
        }
    }

    // ── VFS initialisation ───────────────────────────────────
    unsafe {
        let inodes = &mut *INODES.0.get();
        let ramfs = &mut *RAMFS.0.get();
        inodes.init_root();
        let dev_id = inodes.resolve(microkernel::vfs::ROOT_INODE, "/dev").unwrap_or(microkernel::vfs::NO_INODE);
        if dev_id != microkernel::vfs::NO_INODE {
            inodes.create_device_in(dev_id, "null", 0, 0);
            inodes.create_device_in(dev_id, "zero", 0, 1);
            inodes.create_device_in(dev_id, "console", 0, 2);
            inodes.create_device_in(dev_id, "random", 0, 3);
            inodes.create_device_in(dev_id, "keyboard", 1, 0);
            inodes.create_device_in(dev_id, "mouse", 1, 1);
        }
        let etc_id = inodes.resolve(microkernel::vfs::ROOT_INODE, "/etc").unwrap_or(microkernel::vfs::NO_INODE);
        if etc_id != microkernel::vfs::NO_INODE {
            ramfs.create_with_content(inodes, etc_id, "motd", b"Welcome to VeerOS!\n");
            ramfs.create_with_content(inodes, etc_id, "hostname", b"veeros-raspi5\n");
        }
    }
    let _ = writeln!(con, "[boot] VFS initialised (ramfs {} KiB)", microkernel::ramfs::RAMFS_POOL_SIZE / 1024);

    // ── scheduler + tasks ────────────────────────────────────
    unsafe {
        let sched = &mut *SCHEDULER.0.get();
        let procs = &mut *PROCESSES.0.get();

        // Create process 0 (init/kernel process).
        procs.create("init", usize::MAX, 0, 0);

        // Initialize user table with default accounts.
        let user_tbl = &mut *USERS.0.get();
        user_tbl.init_defaults();

        // Idle task (priority 0).
        let sb = IDLE_STACK.0.as_ptr() as usize;
        let st = sb + IDLE_STACK.0.len();
        if let Some(idx) = sched.create_task("idle", idle_task as *const () as usize, st, sb, 0, 0) {
            sched.tasks[idx].context.set_status(INITIAL_SPSR);
        }

        // Shell task (priority 1).
        #[cfg(feature = "shell")]
        {
            let sb = SHELL_STACK.0.as_ptr() as usize;
            let st = sb + SHELL_STACK.0.len();
            if let Some(idx) = sched.create_task("shell", shell_task as *const () as usize, st, sb, 1, 0) {
                sched.tasks[idx].context.set_status(INITIAL_SPSR);
            }
        }

        // Sample tasks.
        #[cfg(feature = "samples")]
        {
            use crate::samples;

            let sb = HELLO_STACK.0.as_ptr() as usize;
            let st = sb + HELLO_STACK.0.len();
            if let Some(idx) = sched.create_task("hello", samples::hello_task as *const () as usize, st, sb, 2, 0) {
                sched.tasks[idx].context.set_status(INITIAL_SPSR);
            }

            let sb = TIMER_STACK.0.as_ptr() as usize;
            let st = sb + TIMER_STACK.0.len();
            if let Some(idx) = sched.create_task("timer", samples::timer_task as *const () as usize, st, sb, 2, 0) {
                sched.tasks[idx].context.set_status(INITIAL_SPSR);
            }

            let sb = IPC_TX_STACK.0.as_ptr() as usize;
            let st = sb + IPC_TX_STACK.0.len();
            if let Some(idx) = sched.create_task("ipc-tx", samples::ipc_sender_task as *const () as usize, st, sb, 2, 0) {
                sched.tasks[idx].context.set_status(INITIAL_SPSR);
            }

            let sb = IPC_RX_STACK.0.as_ptr() as usize;
            let st = sb + IPC_RX_STACK.0.len();
            if let Some(idx) = sched.create_task("ipc-rx", samples::ipc_receiver_task as *const () as usize, st, sb, 2, 0) {
                sched.tasks[idx].context.set_status(INITIAL_SPSR);
            }
        }
    }

    // Set init process thread count.
    unsafe {
        let sched = &*SCHEDULER.0.get();
        let procs = &mut *PROCESSES.0.get();
        let count = sched.tasks.iter().filter(|t| t.state != TaskState::Free).count();
        procs.processes[0].thread_count = count;
    }

    let _ = writeln!(con, "[boot] idle task registered");
    #[cfg(feature = "shell")]
    let _ = writeln!(con, "[boot] shell task registered");
    #[cfg(feature = "samples")]
    let _ = writeln!(con, "[boot] sample tasks registered (hello, timer, ipc-tx, ipc-rx)");

    // ── snapshot boot log into klog ──────────────────────────
    {
        let klog = unsafe { &mut *KLOG.0.get() };
        let _ = writeln!(klog, "[boot] VeerOS v{VERSION} -- Raspberry Pi 5 (AArch64)");
        let _ = writeln!(klog, "[boot] heap {} KiB, VFS ready, {} drivers",
            HEAP_SIZE / 1024, 3);
        let sd = unsafe { &*SD.0.get() };
        if sd.is_ready() {
            use arch::BlockDevice;
            let _ = writeln!(klog, "[boot] EMMC2 SD: {} sectors", sd.block_count());
        }
        let _ = writeln!(klog, "[boot] GIC-400 + ARM timer initialised");
        let _ = writeln!(klog, "[boot] scheduler starting (preemptive mode)");
    }

    // ── start the first task (never returns) ─────────────────
    unsafe {
        let sched = &mut *SCHEDULER.0.get();
        let _ = writeln!(con, "[boot] starting scheduler -- preemptive mode");
        let ctx_ptr = sched.start().expect("no runnable task");
        let task = &sched.tasks[sched.current];
        let _ = writeln!(con, "[boot] first task: {}", task.name);

        #[cfg(target_arch = "aarch64")]
        {
            extern "C" {
                fn _veer_start_first_task(ctx: *const arch::TaskContext) -> !;
            }
            _veer_start_first_task(ctx_ptr);
        }

        #[cfg(not(target_arch = "aarch64"))]
        {
            let _ = ctx_ptr;
            drop(con);
            #[cfg(feature = "shell")]
            shell_task();
            #[cfg(not(feature = "shell"))]
            idle_task();
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Safe MMIO probe — reads a u32 from an address, returning None on fault
// ═══════════════════════════════════════════════════════════════════════════

/// Try to read a `u32` from `addr`. Returns `Some(value)` if the read
/// succeeded, or `None` if it caused a data abort (unmapped memory).
///
/// Uses the trap handler's probe mechanism: sets PROBE_ACTIVE, does
/// a volatile read, checks PROBE_FAULTED.
#[cfg(target_arch = "aarch64")]
#[allow(dead_code)]
fn safe_read32(addr: usize) -> Option<u32> {
    unsafe {
        trap::PROBE_ACTIVE = true;
        trap::PROBE_FAULTED = false;

        let val: u32;
        core::arch::asm!(
            "ldr {val:w}, [{addr}]",
            addr = in(reg) addr,
            val = out(reg) val,
            options(nostack),
        );

        trap::PROBE_ACTIVE = false;

        if trap::PROBE_FAULTED {
            None
        } else {
            Some(val)
        }
    }
}

/// Print a probe result line.
#[allow(dead_code)]
fn print_probe(con: &mut Console<FbConsole>, label: &str, val: Option<u32>) {
    match val {
        Some(v) => { let _ = writeln!(con, "[boot]   {} = {:#010x}", label, v); }
        None    => { let _ = writeln!(con, "[boot]   {} = FAULT", label); }
    }
}

/// Read a 32-bit value from PCIe config space using BRCM EXT_CFG mechanism.
// (pcie_config_read removed — now in soc_raspi5::pcie module)

// ═══════════════════════════════════════════════════════════════════════════
// Console I/O callbacks (used by the syscall dispatcher)
// ═══════════════════════════════════════════════════════════════════════════

#[allow(dead_code)]
pub(crate) fn console_write_byte(b: u8) {
    use arch::Serial;
    let fbcon = unsafe { *FBCON.0.get() };
    fbcon.write_byte(b);
}

/// Keyboard poll callback — registered with FbConsole so `read_byte()`
/// can pull from the USB HID keyboard queue fed by the timer tick.
fn kbd_poll_fn() -> Option<u8> {
    let input = unsafe { &mut *INPUT.0.get() };
    if !input.active { return None; }
    let mut buf = [0u8; 1];
    if input.kbd_read(&mut buf) > 0 { Some(buf[0]) } else { None }
}

/// Non-consuming check: does the keyboard queue have data?
fn kbd_has_data_fn() -> bool {
    let input = unsafe { &*INPUT.0.get() };
    input.active && input.kbd_has_data()
}

#[allow(dead_code)]
pub(crate) fn console_read_byte() -> u8 {
    use arch::Serial;
    // Check USB keyboard queue first (fed by timer-tick HID poll).
    let input = unsafe { &mut *INPUT.0.get() };
    if input.active {
        let mut buf = [0u8; 1];
        if input.kbd_read(&mut buf) > 0 {
            return buf[0];
        }
    }
    // Fall back to UART.
    let fbcon = unsafe { *FBCON.0.get() };
    fbcon.read_byte()
}

// ═══════════════════════════════════════════════════════════════════════════
// Shell query callbacks (injected into ShellEnv)
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(feature = "shell")]
fn get_uptime_ticks() -> u64 {
    unsafe { (*SCHEDULER.0.get()).ticks }
}

#[cfg(feature = "shell")]
fn write_mem_info(w: &mut dyn core::fmt::Write) {
    unsafe {
        (*HEAP.0.get()).write_stats(w);
    }
}

#[cfg(feature = "shell")]
fn write_driver_list(w: &mut dyn core::fmt::Write) {
    unsafe {
        (*DRIVERS.0.get()).write_list(w);
    }
}

#[cfg(feature = "shell")]
fn write_task_list(w: &mut dyn core::fmt::Write) {
    let sched = unsafe { &*SCHEDULER.0.get() };
    let _ = writeln!(w, "  ID  STATE     PRI  NAME");
    let _ = writeln!(w, "  --  --------  ---  --------");
    for (i, t) in sched.tasks.iter().enumerate() {
        if t.state != TaskState::Free {
            let st = match t.state {
                TaskState::Free => "free",
                TaskState::Ready => "ready",
                TaskState::Running => "RUNNING",
                TaskState::Blocked => "blocked",
                TaskState::Suspended => "suspend",
                TaskState::Zombie => "zombie",
            };
            let _ = writeln!(w, "  {:2}  {:8}  {:3}  {}", i, st, t.priority, t.name);
        }
    }
    let _ = writeln!(w, "  ticks: {}", sched.ticks);
}

#[cfg(feature = "shell")]
fn get_current_user() -> (u16, &'static str) {
    unsafe {
        let users = &*USERS.0.get();
        let uid = (*PROCESSES.0.get()).processes[0].uid;
        (uid, users.name_for_uid(uid))
    }
}

#[cfg(feature = "shell")]
fn write_user_list(w: &mut dyn core::fmt::Write) {
    unsafe {
        let users = &*USERS.0.get();
        let _ = writeln!(w, "  USER     UID  STATUS");
        let _ = writeln!(w, "  -------  ---  ------");
        for i in 0..users.user_count {
            let u = &users.users[i];
            let active = users.sessions.iter().any(|s| s.active && s.uid == u.uid);
            let st = if active { "active" } else { "      " };
            let _ = writeln!(w, "  {:7}  {:3}  {}", u.name, u.uid, st);
        }
    }
}

// ---------------------------------------------------------------------------
// Multi-user callbacks
// ---------------------------------------------------------------------------

#[cfg(all(feature = "shell", feature = "multi-user"))]
fn do_login(username: &str, password: &[u8]) -> u32 {
    unsafe {
        let users = &mut *USERS.0.get();
        match users.login(username, password) {
            Ok(token) => {
                if let Some((uid, gid)) = users.session_info(token) {
                    (*PROCESSES.0.get()).processes[0].uid = uid;
                    (*PROCESSES.0.get()).processes[0].gid = gid;
                }
                token
            }
            Err(_) => 0,
        }
    }
}

#[cfg(all(feature = "shell", feature = "multi-user"))]
fn do_logout() -> bool {
    unsafe {
        let users = &mut *USERS.0.get();
        let uid = (*PROCESSES.0.get()).processes[0].uid;
        let mut found = false;
        for s in users.sessions.iter_mut() {
            if s.active && s.uid == uid {
                *s = microkernel::user::Session::empty();
                found = true;
                break;
            }
        }
        (*PROCESSES.0.get()).processes[0].uid = microkernel::user::ROOT_UID;
        (*PROCESSES.0.get()).processes[0].gid = microkernel::user::ROOT_GID;
        found
    }
}

#[cfg(all(feature = "shell", feature = "multi-user"))]
fn do_change_password(uid: u16, new_password: &[u8]) -> bool {
    unsafe { (*USERS.0.get()).change_password(uid, new_password) }
}

#[cfg(all(feature = "shell", feature = "multi-user"))]
fn do_add_user(name: &'static str, gid: u16, password: &[u8]) -> Option<u16> {
    unsafe { (*USERS.0.get()).add_user(name, gid, password) }
}

#[cfg(all(feature = "shell", feature = "multi-user"))]
fn do_remove_user(uid: u16) -> bool {
    unsafe { (*USERS.0.get()).remove_user(uid) }
}

// ---------------------------------------------------------------------------
// VFS callbacks (injected into ShellEnv)
// ---------------------------------------------------------------------------

#[cfg(feature = "shell")]
fn vfs_list_dir(path: &str, w: &mut dyn core::fmt::Write) {
    use microkernel::vfs::{InodeKind, NO_INODE};
    unsafe {
        let inodes = &*INODES.0.get();
        let cwd = (*PROCESSES.0.get()).processes[0].cwd;
        let dir_id = if path == "." { Some(cwd) } else { inodes.resolve(cwd, path) };
        match dir_id {
            Some(id) if id != NO_INODE => {
                let inode = &inodes.inodes[id as usize];
                if inode.kind != InodeKind::Directory {
                    let _ = writeln!(w, "ls: '{}': not a directory", path);
                    return;
                }
                let mut child = inode.children_head;
                while child != NO_INODE {
                    let c = &inodes.inodes[child as usize];
                    let kind_ch = match c.kind {
                        InodeKind::Directory => 'd',
                        InodeKind::File => 'f',
                        InodeKind::Device => 'c',
                        _ => '?',
                    };
                    let _ = writeln!(w, "  {}  {:6}  {}", kind_ch, c.size, c.name_str());
                    child = c.next_sibling;
                }
            }
            _ => { let _ = writeln!(w, "ls: '{}': no such directory", path); }
        }
    }
}

#[cfg(feature = "shell")]
fn vfs_read_file(path: &str, buf: &mut [u8]) -> usize {
    use microkernel::vfs::{InodeKind, NO_INODE};
    unsafe {
        let inodes = &*INODES.0.get();
        let ramfs = &*RAMFS.0.get();
        let cwd = (*PROCESSES.0.get()).processes[0].cwd;
        let id = inodes.resolve(cwd, path).unwrap_or(NO_INODE);
        if id == NO_INODE { return 0; }
        if inodes.inodes[id as usize].kind != InodeKind::File { return 0; }
        ramfs.read(inodes, id, 0, buf)
    }
}

#[cfg(feature = "shell")]
fn vfs_write_file(path: &str, data: &[u8], append: bool) -> bool {
    use microkernel::vfs::{InodeKind, NO_INODE};
    unsafe {
        let inodes = &mut *INODES.0.get();
        let ramfs = &mut *RAMFS.0.get();
        let cwd = (*PROCESSES.0.get()).processes[0].cwd;
        let mut id = inodes.resolve(cwd, path).unwrap_or(NO_INODE);
        if id == NO_INODE {
            if let Some(slash) = path.rfind('/') {
                let parent_path = if slash == 0 { "/" } else { &path[..slash] };
                let name = &path[slash + 1..];
                let parent = inodes.resolve(cwd, parent_path).unwrap_or(NO_INODE);
                if parent == NO_INODE || name.is_empty() { return false; }
                id = match inodes.create_file_in(parent, name) { Some(i) => i, None => return false };
            } else {
                id = match inodes.create_file_in(cwd, path) { Some(i) => i, None => return false };
            }
        }
        if inodes.inodes[id as usize].kind != InodeKind::File { return false; }
        let offset = if append { inodes.inodes[id as usize].size } else { 0 };
        if !append { ramfs.truncate(inodes, id, 0); }
        ramfs.write(inodes, id, offset, data) > 0
    }
}

#[cfg(feature = "shell")]
fn vfs_mkdir(path: &str) -> bool {
    use microkernel::vfs::NO_INODE;
    unsafe {
        let inodes = &mut *INODES.0.get();
        let cwd = (*PROCESSES.0.get()).processes[0].cwd;
        if let Some(slash) = path.rfind('/') {
            let parent_path = if slash == 0 { "/" } else { &path[..slash] };
            let name = &path[slash + 1..];
            let parent = inodes.resolve(cwd, parent_path).unwrap_or(NO_INODE);
            if parent == NO_INODE || name.is_empty() { return false; }
            inodes.mkdir_in(parent, name).is_some()
        } else {
            inodes.mkdir_in(cwd, path).is_some()
        }
    }
}

#[cfg(feature = "shell")]
fn vfs_stat(path: &str, w: &mut dyn core::fmt::Write) {
    use microkernel::vfs::{InodeKind, NO_INODE};
    unsafe {
        let inodes = &*INODES.0.get();
        let cwd = (*PROCESSES.0.get()).processes[0].cwd;
        let id = inodes.resolve(cwd, path).unwrap_or(NO_INODE);
        if id == NO_INODE {
            let _ = writeln!(w, "stat: '{}': no such file or directory", path);
            return;
        }
        let inode = &inodes.inodes[id as usize];
        let kind = match inode.kind { InodeKind::File => "file", InodeKind::Directory => "directory", InodeKind::Device => "device", _ => "unknown" };
        let _ = writeln!(w, "  File: {}", inode.name_str());
        let _ = writeln!(w, "  Type: {}", kind);
        let _ = writeln!(w, "  Size: {}", inode.size);
        let _ = writeln!(w, "  Inode: {}", id);
        let _ = writeln!(w, "  Parent: {}", inode.parent);
        if inode.kind == InodeKind::Device {
            let _ = writeln!(w, "  Device: {},{}", inode.dev_major, inode.dev_minor);
        }
    }
}

#[cfg(feature = "shell")]
fn vfs_unlink(path: &str) -> bool {
    use microkernel::vfs::NO_INODE;
    unsafe {
        let inodes = &mut *INODES.0.get();
        let cwd = (*PROCESSES.0.get()).processes[0].cwd;
        let id = inodes.resolve(cwd, path).unwrap_or(NO_INODE);
        if id == NO_INODE { return false; }
        inodes.unlink(id)
    }
}

#[cfg(feature = "shell")]
fn vfs_rename(old: &str, new: &str) -> bool {
    use microkernel::vfs::NO_INODE;
    unsafe {
        let inodes = &mut *INODES.0.get();
        let cwd = (*PROCESSES.0.get()).processes[0].cwd;
        let id = inodes.resolve(cwd, old).unwrap_or(NO_INODE);
        if id == NO_INODE { return false; }
        if let Some(slash) = new.rfind('/') {
            let parent_path = if slash == 0 { "/" } else { &new[..slash] };
            let name = &new[slash + 1..];
            let parent = inodes.resolve(cwd, parent_path).unwrap_or(NO_INODE);
            if parent == NO_INODE || name.is_empty() { return false; }
            inodes.rename(id, parent, name)
        } else {
            inodes.rename(id, cwd, new)
        }
    }
}

#[cfg(feature = "shell")]
fn vfs_getcwd(buf: &mut [u8]) -> usize {
    unsafe {
        let inodes = &*INODES.0.get();
        let cwd = (*PROCESSES.0.get()).processes[0].cwd;
        inodes.build_path(cwd, buf)
    }
}

#[cfg(feature = "shell")]
fn vfs_chdir(path: &str) -> bool {
    use microkernel::vfs::{InodeKind, NO_INODE};
    unsafe {
        let inodes = &*INODES.0.get();
        let procs = &mut *PROCESSES.0.get();
        let cwd = procs.processes[0].cwd;
        let id = inodes.resolve(cwd, path).unwrap_or(NO_INODE);
        if id == NO_INODE { return false; }
        if inodes.inodes[id as usize].kind != InodeKind::Directory { return false; }
        procs.processes[0].cwd = id;
        true
    }
}

#[cfg(feature = "shell")]
fn vfs_tree(path: &str, w: &mut dyn core::fmt::Write) {
    use microkernel::vfs::{InodeKind, ROOT_INODE, NO_INODE};
    unsafe {
        let inodes = &*INODES.0.get();
        let cwd = (*PROCESSES.0.get()).processes[0].cwd;
        let start = if path == "/" { ROOT_INODE } else { inodes.resolve(cwd, path).unwrap_or(NO_INODE) };
        if start == NO_INODE { let _ = writeln!(w, "tree: '{}': no such directory", path); return; }
        let root = &inodes.inodes[start as usize];
        if root.kind != InodeKind::Directory { let _ = writeln!(w, "tree: '{}': not a directory", path); return; }
        let _ = writeln!(w, "{}", if path == "/" || path == "." { "/" } else { path });
        let mut stack: [(u16, u8); 64] = [(NO_INODE, 0); 64];
        let mut sp = 0usize;
        let mut kids: [u16; 64] = [NO_INODE; 64];
        let mut nk = 0usize;
        let mut ch = root.children_head;
        while ch != NO_INODE && nk < 64 { kids[nk] = ch; nk += 1; ch = inodes.inodes[ch as usize].next_sibling; }
        let mut i = nk;
        while i > 0 { i -= 1; if sp < 64 { stack[sp] = (kids[i], 1); sp += 1; } }
        while sp > 0 {
            sp -= 1;
            let (id, depth) = stack[sp];
            let node = &inodes.inodes[id as usize];
            for _ in 0..depth { w.write_str("  ").ok(); }
            let kind_ch = match node.kind { InodeKind::Directory => '/', InodeKind::Device => '*', _ => ' ' };
            let _ = writeln!(w, "{}{}", node.name_str(), kind_ch);
            if node.kind == InodeKind::Directory {
                let mut ck: [u16; 64] = [NO_INODE; 64];
                let mut cn = 0usize;
                let mut c = node.children_head;
                while c != NO_INODE && cn < 64 { ck[cn] = c; cn += 1; c = inodes.inodes[c as usize].next_sibling; }
                let mut j = cn;
                while j > 0 { j -= 1; if sp < 64 { stack[sp] = (ck[j], depth + 1); sp += 1; } }
            }
        }
    }
}

#[cfg(feature = "shell")]
fn vfs_touch(path: &str) -> bool {
    use microkernel::vfs::NO_INODE;
    unsafe {
        let inodes = &mut *INODES.0.get();
        let cwd = (*PROCESSES.0.get()).processes[0].cwd;
        if inodes.resolve(cwd, path).unwrap_or(NO_INODE) != NO_INODE { return true; }
        if let Some(slash) = path.rfind('/') {
            let parent_path = if slash == 0 { "/" } else { &path[..slash] };
            let name = &path[slash + 1..];
            let parent = inodes.resolve(cwd, parent_path).unwrap_or(NO_INODE);
            if parent == NO_INODE || name.is_empty() { return false; }
            inodes.create_file_in(parent, name).is_some()
        } else {
            inodes.create_file_in(cwd, path).is_some()
        }
    }
}

fn mount_list(w: &mut dyn core::fmt::Write) {
    unsafe {
        let mounts = &*MOUNTS.0.get();
        let inodes = &*INODES.0.get();
        let mut found = false;
        for (i, m) in mounts.mounts.iter().enumerate() {
            if m.active {
                let mut pathbuf = [0u8; 64];
                let plen = inodes.build_path(m.dir_inode, &mut pathbuf);
                let path = core::str::from_utf8(&pathbuf[..plen]).unwrap_or("?");
                let fstype = match m.fs_type {
                    microkernel::vfs::FsType::Fat32 => "fat32",
                    microkernel::vfs::FsType::RamFs => "ramfs",
                    _ => "none",
                };
                let _ = writeln!(w, "  {} on {} type {} (slot {})", m.label_str(), path, fstype, i + 1);
                found = true;
            }
        }
        if !found {
            let _ = writeln!(w, "  (no filesystems mounted)");
        }
    }
}

fn lsblk_info(w: &mut dyn core::fmt::Write) {
    let _ = writeln!(w, "  NAME       TYPE   SIZE");
    let sd = unsafe { &*SD.0.get() };
    if sd.is_ready() {
        use arch::BlockDevice;
        let sectors = sd.block_count();
        let mb = sectors / 2048; // 512 bytes/sector, 2048 sectors/MiB
        let _ = writeln!(w, "  emmc2-sd   disk   {} MiB ({} sectors)", mb, sectors);
    } else {
        let _ = writeln!(w, "  emmc2-sd   disk   (not detected)");
    }
}

fn input_status(w: &mut dyn core::fmt::Write) {
    let input = unsafe { &*INPUT.0.get() };
    input.write_status(w);
}

fn usb_list(w: &mut dyn core::fmt::Write) {
    let xhci0 = unsafe { &*XHCI0.0.get() };
    let _ = writeln!(w, "--- xHCI0 (RP1 USB 3.0) ---");
    xhci0.write_port_list(w);
    xhci0.write_device_list(w);
    let xhci1 = unsafe { &*XHCI1.0.get() };
    let _ = writeln!(w, "--- xHCI1 (RP1 USB 2.0) ---");
    xhci1.write_port_list(w);
    xhci1.write_device_list(w);
}

// ─── GPIO command callback ───────────────────────────────────────────────

fn gpio_cmd(sub: &str, args: &str, w: &mut dyn core::fmt::Write) {
    match sub {
        "list" | "" => {
            soc_raspi5::gpio::write_pin_list(w);
        }
        "read" => {
            if let Ok(pin) = parse_u8(args.trim()) {
                if pin > 27 {
                    let _ = writeln!(w, "  error: pin must be 0–27");
                } else {
                    let val = soc_raspi5::gpio::read(pin);
                    let _ = writeln!(w, "  GPIO{} = {}", pin, if val { 1 } else { 0 });
                }
            } else {
                let _ = writeln!(w, "  usage: gpio read <pin>");
            }
        }
        "write" => {
            let (pin_s, val_s) = split_at_space(args);
            if let (Ok(pin), Ok(val)) = (parse_u8(pin_s), parse_u8(val_s)) {
                if pin > 27 {
                    let _ = writeln!(w, "  error: pin must be 0–27");
                } else {
                    soc_raspi5::gpio::write(pin, val != 0);
                    let _ = writeln!(w, "  GPIO{} <- {}", pin, if val != 0 { 1 } else { 0 });
                }
            } else {
                let _ = writeln!(w, "  usage: gpio write <pin> <0|1>");
            }
        }
        "mode" => {
            let (pin_s, mode_s) = split_at_space(args);
            if let Ok(pin) = parse_u8(pin_s) {
                if pin > 27 {
                    let _ = writeln!(w, "  error: pin must be 0–27");
                    return;
                }
                match mode_s.trim() {
                    "in" | "input" => {
                        soc_raspi5::gpio::set_mode(pin, soc_raspi5::gpio::GpioMode::Input);
                        let _ = writeln!(w, "  GPIO{} -> INPUT", pin);
                    }
                    "out" | "output" => {
                        soc_raspi5::gpio::set_mode(pin, soc_raspi5::gpio::GpioMode::Output);
                        let _ = writeln!(w, "  GPIO{} -> OUTPUT", pin);
                    }
                    _ => {
                        let _ = writeln!(w, "  usage: gpio mode <pin> <in|out>");
                    }
                }
            } else {
                let _ = writeln!(w, "  usage: gpio mode <pin> <in|out>");
            }
        }
        "pull" => {
            let (pin_s, pull_s) = split_at_space(args);
            if let Ok(pin) = parse_u8(pin_s) {
                if pin > 27 {
                    let _ = writeln!(w, "  error: pin must be 0–27");
                    return;
                }
                match pull_s.trim() {
                    "none" | "off" => {
                        soc_raspi5::gpio::set_pull(pin, soc_raspi5::gpio::GpioPull::None);
                        let _ = writeln!(w, "  GPIO{} pull -> NONE", pin);
                    }
                    "up" => {
                        soc_raspi5::gpio::set_pull(pin, soc_raspi5::gpio::GpioPull::Up);
                        let _ = writeln!(w, "  GPIO{} pull -> UP", pin);
                    }
                    "down" => {
                        soc_raspi5::gpio::set_pull(pin, soc_raspi5::gpio::GpioPull::Down);
                        let _ = writeln!(w, "  GPIO{} pull -> DOWN", pin);
                    }
                    _ => {
                        let _ = writeln!(w, "  usage: gpio pull <pin> <none|up|down>");
                    }
                }
            } else {
                let _ = writeln!(w, "  usage: gpio pull <pin> <none|up|down>");
            }
        }
        "toggle" => {
            if let Ok(pin) = parse_u8(args.trim()) {
                if pin > 27 {
                    let _ = writeln!(w, "  error: pin must be 0–27");
                } else {
                    soc_raspi5::gpio::toggle(pin);
                    let _ = writeln!(w, "  GPIO{} toggled", pin);
                }
            } else {
                let _ = writeln!(w, "  usage: gpio toggle <pin>");
            }
        }
        _ => {
            let _ = writeln!(w, "  gpio subcommands: list, read, write, mode, pull, toggle");
        }
    }
}

// ─── I2C command callback ────────────────────────────────────────────────

fn i2c_cmd(sub: &str, args: &str, w: &mut dyn core::fmt::Write) {
    match sub {
        "scan" => {
            let bus_n = if args.trim().is_empty() { 1u8 } else {
                parse_u8(args.trim()).unwrap_or(1)
            };
            if bus_n > 6 {
                let _ = writeln!(w, "  error: bus must be 0–6");
                return;
            }
            let _ = writeln!(w, "  Scanning I2C bus {}...", bus_n);
            let _ = writeln!(w, "     0  1  2  3  4  5  6  7  8  9  a  b  c  d  e  f");
            let i2c = match bus_n {
                0 => soc_raspi5::i2c::Rp1I2c::i2c0(),
                1 => soc_raspi5::i2c::Rp1I2c::i2c1(),
                2 => soc_raspi5::i2c::Rp1I2c::i2c2(),
                3 => soc_raspi5::i2c::Rp1I2c::i2c3(),
                4 => soc_raspi5::i2c::Rp1I2c::i2c4(),
                5 => soc_raspi5::i2c::Rp1I2c::i2c5(),
                6 => soc_raspi5::i2c::Rp1I2c::i2c6(),
                _ => return,
            };
            i2c.init(soc_raspi5::i2c::I2cSpeed::Standard);
            for row in 0..8u8 {
                let _ = write!(w, "  {:02x}:", row * 16);
                for col in 0..16u8 {
                    let addr = row * 16 + col;
                    if addr < 0x03 || addr > 0x77 {
                        let _ = write!(w, "   ");
                    } else {
                        let mut buf = [0u8; 1];
                        match i2c.read_from(addr, &mut buf) {
                            Ok(_) => { let _ = write!(w, " {:02x}", addr); }
                            Err(_) => { let _ = write!(w, " --"); }
                        }
                    }
                }
                let _ = writeln!(w);
            }
        }
        "read" => {
            // i2c read <bus> <addr> <reg>
            let parts: [&str; 3] = parse_args_3(args);
            if let (Ok(bus), Ok(addr), Ok(reg)) = (
                parse_u8(parts[0]), parse_hex_u8(parts[1]), parse_hex_u8(parts[2])
            ) {
                if bus > 6 { let _ = writeln!(w, "  error: bus 0–6"); return; }
                let i2c = make_i2c(bus);
                i2c.init(soc_raspi5::i2c::I2cSpeed::Standard);
                let mut buf = [0u8; 1];
                match i2c.write_read(addr, &[reg], &mut buf) {
                    Ok(_) => { let _ = writeln!(w, "  bus {} addr 0x{:02X} reg 0x{:02X} = 0x{:02X}", bus, addr, reg, buf[0]); }
                    Err(e) => { let _ = writeln!(w, "  error: {:?}", e); }
                }
            } else {
                let _ = writeln!(w, "  usage: i2c read <bus> <addr> <reg>");
            }
        }
        "write" => {
            // i2c write <bus> <addr> <reg> <val>
            let parts: [&str; 4] = parse_args_4(args);
            if let (Ok(bus), Ok(addr), Ok(reg), Ok(val)) = (
                parse_u8(parts[0]), parse_hex_u8(parts[1]),
                parse_hex_u8(parts[2]), parse_hex_u8(parts[3])
            ) {
                if bus > 6 { let _ = writeln!(w, "  error: bus 0–6"); return; }
                let i2c = make_i2c(bus);
                i2c.init(soc_raspi5::i2c::I2cSpeed::Standard);
                match i2c.write_to(addr, &[reg, val]) {
                    Ok(_) => { let _ = writeln!(w, "  OK: wrote 0x{:02X} to reg 0x{:02X} on 0x{:02X}", val, reg, addr); }
                    Err(e) => { let _ = writeln!(w, "  error: {:?}", e); }
                }
            } else {
                let _ = writeln!(w, "  usage: i2c write <bus> <addr> <reg> <val>");
            }
        }
        _ => {
            let _ = writeln!(w, "  i2c subcommands: scan, read, write");
        }
    }
}

fn make_i2c(bus: u8) -> soc_raspi5::i2c::Rp1I2c {
    match bus {
        0 => soc_raspi5::i2c::Rp1I2c::i2c0(),
        1 => soc_raspi5::i2c::Rp1I2c::i2c1(),
        2 => soc_raspi5::i2c::Rp1I2c::i2c2(),
        3 => soc_raspi5::i2c::Rp1I2c::i2c3(),
        4 => soc_raspi5::i2c::Rp1I2c::i2c4(),
        5 => soc_raspi5::i2c::Rp1I2c::i2c5(),
        6 => soc_raspi5::i2c::Rp1I2c::i2c6(),
        _ => soc_raspi5::i2c::Rp1I2c::i2c1(),
    }
}

// ─── SPI command callback ────────────────────────────────────────────────

fn spi_cmd(sub: &str, args: &str, w: &mut dyn core::fmt::Write) {
    match sub {
        "cfg" => {
            // spi cfg <bus> <mode> <freq_div>
            let parts: [&str; 3] = parse_args_3(args);
            if let (Ok(bus), Ok(mode), Ok(div)) = (
                parse_u8(parts[0]), parse_u8(parts[1]), parse_u16(parts[2])
            ) {
                if bus > 5 { let _ = writeln!(w, "  error: bus 0–5"); return; }
                if mode > 3 { let _ = writeln!(w, "  error: mode 0–3"); return; }
                let spi_mode = match mode {
                    0 => soc_raspi5::spi::SpiMode::Mode0,
                    1 => soc_raspi5::spi::SpiMode::Mode1,
                    2 => soc_raspi5::spi::SpiMode::Mode2,
                    _ => soc_raspi5::spi::SpiMode::Mode3,
                };
                let spi = make_spi(bus);
                spi.init(spi_mode, div, 8);
                let _ = writeln!(w, "  SPI{} configured: mode {}, divisor {}", bus, mode, div);
            } else {
                let _ = writeln!(w, "  usage: spi cfg <bus> <mode> <freq_div>");
            }
        }
        "xfer" => {
            // spi xfer <bus> <hex bytes...>
            let (bus_s, hex_s) = split_at_space(args);
            if let Ok(bus) = parse_u8(bus_s) {
                if bus > 5 { let _ = writeln!(w, "  error: bus 0–5"); return; }
                let mut tx = [0u8; 32];
                let mut rx = [0u8; 32];
                let mut len = 0usize;
                for tok in hex_s.split_ascii_whitespace() {
                    if len >= 32 { break; }
                    if let Ok(b) = parse_hex_u8(tok) {
                        tx[len] = b;
                        len += 1;
                    }
                }
                if len == 0 {
                    let _ = writeln!(w, "  usage: spi xfer <bus> <hex bytes...>");
                    return;
                }
                let spi = make_spi(bus);
                spi.transfer(&tx[..len], &mut rx[..len]);
                let _ = write!(w, "  TX:");
                for i in 0..len { let _ = write!(w, " {:02X}", tx[i]); }
                let _ = writeln!(w);
                let _ = write!(w, "  RX:");
                for i in 0..len { let _ = write!(w, " {:02X}", rx[i]); }
                let _ = writeln!(w);
            } else {
                let _ = writeln!(w, "  usage: spi xfer <bus> <hex bytes...>");
            }
        }
        _ => {
            let _ = writeln!(w, "  spi subcommands: cfg, xfer");
        }
    }
}

fn make_spi(bus: u8) -> soc_raspi5::spi::Rp1Spi {
    match bus {
        0 => soc_raspi5::spi::Rp1Spi::spi0(),
        1 => soc_raspi5::spi::Rp1Spi::spi1(),
        2 => soc_raspi5::spi::Rp1Spi::spi2(),
        3 => soc_raspi5::spi::Rp1Spi::spi3(),
        4 => soc_raspi5::spi::Rp1Spi::spi4(),
        5 => soc_raspi5::spi::Rp1Spi::spi5(),
        _ => soc_raspi5::spi::Rp1Spi::spi0(),
    }
}

// ─── Hardware info callback ──────────────────────────────────────────────

fn hw_info(w: &mut dyn core::fmt::Write) {
    soc_raspi5::board::write_hw_info(w);
}

fn get_temp_millic() -> i32 {
    soc_raspi5::board::get_temperature()
}

// ─── Kernel log callback ────────────────────────────────────────────────

fn dmesg_info(w: &mut dyn core::fmt::Write) {
    let klog = unsafe { &*KLOG.0.get() };
    klog.dump(w);
}

// ─── Reboot / shutdown ──────────────────────────────────────────────────

fn do_reboot() {
    soc_raspi5::board::reboot();
}

fn do_shutdown() {
    soc_raspi5::board::shutdown();
}

// ─── Arg parsing helpers ────────────────────────────────────────────────

fn split_at_space(s: &str) -> (&str, &str) {
    let s = s.trim();
    match s.find(' ') {
        Some(i) => (s[..i].trim(), s[i + 1..].trim()),
        None => (s, ""),
    }
}

fn parse_u8(s: &str) -> Result<u8, ()> {
    let s = s.trim();
    if s.is_empty() { return Err(()); }
    if s.starts_with("0x") || s.starts_with("0X") {
        parse_hex_u8(s)
    } else {
        let mut val: u16 = 0;
        for &b in s.as_bytes() {
            if b < b'0' || b > b'9' { return Err(()); }
            val = val * 10 + (b - b'0') as u16;
            if val > 255 { return Err(()); }
        }
        Ok(val as u8)
    }
}

fn parse_u16(s: &str) -> Result<u16, ()> {
    let s = s.trim();
    if s.is_empty() { return Err(()); }
    let mut val: u32 = 0;
    for &b in s.as_bytes() {
        if b < b'0' || b > b'9' { return Err(()); }
        val = val * 10 + (b - b'0') as u32;
        if val > 65535 { return Err(()); }
    }
    Ok(val as u16)
}

fn parse_hex_u8(s: &str) -> Result<u8, ()> {
    let s = s.trim();
    let s = if s.starts_with("0x") || s.starts_with("0X") { &s[2..] } else { s };
    if s.is_empty() || s.len() > 2 { return Err(()); }
    let mut val: u8 = 0;
    for &b in s.as_bytes() {
        let nib = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => return Err(()),
        };
        val = val * 16 + nib;
    }
    Ok(val)
}

fn parse_args_3(s: &str) -> [&str; 3] {
    let mut result = [""; 3];
    let mut rest = s.trim();
    for slot in result.iter_mut().take(3) {
        let (a, b) = split_at_space(rest);
        *slot = a;
        rest = b;
    }
    result
}

fn parse_args_4(s: &str) -> [&str; 4] {
    let mut result = [""; 4];
    let mut rest = s.trim();
    for slot in result.iter_mut().take(4) {
        let (a, b) = split_at_space(rest);
        *slot = a;
        rest = b;
    }
    result
}