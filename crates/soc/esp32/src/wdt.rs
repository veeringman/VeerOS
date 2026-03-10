//! Watchdog timer disable for ESP32 RISC-V variants.
//!
//! The ROM bootloader enables multiple watchdog timers by default. If they
//! aren't disabled or fed, the chip resets within a few seconds. Call
//! [`disable_watchdogs()`] early in boot, before any long initialisation.
//!
//! Register addresses differ between C3, C6, and H2 — all handled here
//! via `#[cfg(feature = "...")]` gates.

// ═══════════════════════════════════════════════════════════════════════════
// Variant-specific register addresses
// ═══════════════════════════════════════════════════════════════════════════

// -- TIMG0 WDT base -------------------------------------------------------

#[cfg(feature = "c3")]
const TIMG0_BASE: usize = 0x6001_F000;

#[cfg(feature = "c6")]
const TIMG0_BASE: usize = 0x6000_8000;

#[cfg(feature = "h2")]
const TIMG0_BASE: usize = 0x6000_7000;

#[cfg(all(not(feature = "c3"), not(feature = "c6"), not(feature = "h2")))]
const TIMG0_BASE: usize = 0x6001_F000;

// -- TIMG0 WDT register offsets (same across all variants) ----------------

const TIMG0_WDTCONFIG0: usize = TIMG0_BASE + 0x48;
const TIMG0_WDTWPROTECT: usize = TIMG0_BASE + 0x64;

// -- RTC / LP base ---------------------------------------------------------
// C3: RTC_CNTL at 0x6000_8000
// C6: LP_WDT  at 0x600B_1C00  (low-power WDT replaces RTC_CNTL WDT)
// H2: LP_WDT  at 0x600B_1C00

#[cfg(feature = "c3")]
const RTC_BASE: usize = 0x6000_8000;

#[cfg(feature = "c6")]
const RTC_BASE: usize = 0x600B_1C00;

#[cfg(feature = "h2")]
const RTC_BASE: usize = 0x600B_1C00;

#[cfg(all(not(feature = "c3"), not(feature = "c6"), not(feature = "h2")))]
const RTC_BASE: usize = 0x6000_8000;

// -- RTC / LP WDT register offsets -----------------------------------------
// C3:  WDTCONFIG0 = base+0x90, WDTWPROTECT = base+0xA8
// C6:  WDTCONFIG0 = base+0x00, WDTWPROTECT = base+0x18  (LP_WDT layout)
// H2:  same as C6

#[cfg(feature = "c3")]
const RTC_WDTCONFIG0: usize = RTC_BASE + 0x0090;
#[cfg(feature = "c3")]
const RTC_WDTWPROTECT: usize = RTC_BASE + 0x00A8;

#[cfg(feature = "c6")]
const RTC_WDTCONFIG0: usize = RTC_BASE + 0x0000;
#[cfg(feature = "c6")]
const RTC_WDTWPROTECT: usize = RTC_BASE + 0x0018;

#[cfg(feature = "h2")]
const RTC_WDTCONFIG0: usize = RTC_BASE + 0x0000;
#[cfg(feature = "h2")]
const RTC_WDTWPROTECT: usize = RTC_BASE + 0x0018;

#[cfg(all(not(feature = "c3"), not(feature = "c6"), not(feature = "h2")))]
const RTC_WDTCONFIG0: usize = RTC_BASE + 0x0090;
#[cfg(all(not(feature = "c3"), not(feature = "c6"), not(feature = "h2")))]
const RTC_WDTWPROTECT: usize = RTC_BASE + 0x00A8;

// -- Super Watchdog (SWD) --------------------------------------------------
// C3:  SWD_CONF = RTC_BASE+0xAC, SWD_WPROTECT = RTC_BASE+0xB0
// C6:  SWD lives in LP_WDT block (offset layout differs)
// H2:  same as C6

#[cfg(feature = "c3")]
const SWD_CONF: usize = 0x6000_8000 + 0x00AC;
#[cfg(feature = "c3")]
const SWD_WPROTECT: usize = 0x6000_8000 + 0x00B0;

#[cfg(feature = "c6")]
const SWD_CONF: usize = 0x600B_1C00 + 0x001C;
#[cfg(feature = "c6")]
const SWD_WPROTECT: usize = 0x600B_1C00 + 0x0020;

#[cfg(feature = "h2")]
const SWD_CONF: usize = 0x600B_1C00 + 0x001C;
#[cfg(feature = "h2")]
const SWD_WPROTECT: usize = 0x600B_1C00 + 0x0020;

#[cfg(all(not(feature = "c3"), not(feature = "c6"), not(feature = "h2")))]
const SWD_CONF: usize = 0x6000_8000 + 0x00AC;
#[cfg(all(not(feature = "c3"), not(feature = "c6"), not(feature = "h2")))]
const SWD_WPROTECT: usize = 0x6000_8000 + 0x00B0;

// -- Unlock keys (shared across all variants) ------------------------------

const WDT_UNLOCK_KEY: u32 = 0x50D8_3AA1;
const SWD_UNLOCK_KEY: u32 = 0x8F1D_312A;

// ═══════════════════════════════════════════════════════════════════════════
// Public API
// ═══════════════════════════════════════════════════════════════════════════

/// Disable all hardware watchdog timers (TIMG0 WDT, RTC/LP WDT, Super WDT).
///
/// Must be called very early in boot — before any long initialisation that
/// could exceed the default watchdog timeout (~3-5 seconds).
pub fn disable_watchdogs() {
    // ── TIMG0 WDT ────────────────────────────────────────────
    unsafe {
        core::ptr::write_volatile(TIMG0_WDTWPROTECT as *mut u32, WDT_UNLOCK_KEY);
        core::ptr::write_volatile(TIMG0_WDTCONFIG0 as *mut u32, 0);
        core::ptr::write_volatile(TIMG0_WDTWPROTECT as *mut u32, 0);
    }

    // ── RTC / LP WDT ─────────────────────────────────────────
    unsafe {
        core::ptr::write_volatile(RTC_WDTWPROTECT as *mut u32, WDT_UNLOCK_KEY);
        core::ptr::write_volatile(RTC_WDTCONFIG0 as *mut u32, 0);
        core::ptr::write_volatile(RTC_WDTWPROTECT as *mut u32, 0);
    }

    // ── Super Watchdog (auto-feed to effectively disable) ────
    unsafe {
        core::ptr::write_volatile(SWD_WPROTECT as *mut u32, SWD_UNLOCK_KEY);
        let conf = core::ptr::read_volatile(SWD_CONF as *const u32);
        // Set SWD_AUTO_FEED_EN (bit 30 on C3, bit 31 on C6/H2).
        #[cfg(feature = "c3")]
        let new_conf = conf | (1 << 30);
        #[cfg(any(feature = "c6", feature = "h2"))]
        let new_conf = conf | (1 << 31);
        #[cfg(all(not(feature = "c3"), not(feature = "c6"), not(feature = "h2")))]
        let new_conf = conf | (1 << 30);
        core::ptr::write_volatile(SWD_CONF as *mut u32, new_conf);
        core::ptr::write_volatile(SWD_WPROTECT as *mut u32, 0);
    }
}
