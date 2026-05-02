//! Driver isolation model for VeerOS.
//!
//! Each device driver registers itself with the kernel, declaring which
//! capabilities it needs (MMIO regions, interrupt lines, DMA).
//! The registry enforces that drivers only access resources they were
//! granted, providing a fault boundary without an MMU.
//!
//! This is a *software* isolation model — it catches programming errors
//! and limits blast radius, but cannot prevent a truly malicious driver
//! in M-mode.  On chips with PMP (Physical Memory Protection) the
//! granted regions can additionally be enforced in hardware.

use core::fmt;

// ---------------------------------------------------------------------------
// Driver capabilities
// ---------------------------------------------------------------------------

/// Permissions a driver may request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriverCaps {
    /// Number of MMIO regions this driver needs.
    pub mmio_regions: u8,
    /// The driver uses interrupt lines.
    pub uses_interrupts: bool,
    /// The driver performs DMA (needs contiguous physical buffers).
    pub uses_dma: bool,
    /// The driver accesses the network stack.
    pub uses_network: bool,
}

impl DriverCaps {
    pub const fn none() -> Self {
        Self {
            mmio_regions: 0,
            uses_interrupts: false,
            uses_dma: false,
            uses_network: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Memory region descriptor
// ---------------------------------------------------------------------------

/// A bounded memory region granted to a driver.
#[derive(Debug, Clone, Copy)]
pub struct MemRegion {
    pub base: usize,
    pub size: usize,
}

impl MemRegion {
    pub const fn new(base: usize, size: usize) -> Self {
        Self { base, size }
    }

    /// Returns `true` if `addr` falls within this region.
    pub fn contains(&self, addr: usize) -> bool {
        addr >= self.base && addr < self.base + self.size
    }

    /// Returns `true` if the range `[addr, addr+len)` is fully inside.
    pub fn contains_range(&self, addr: usize, len: usize) -> bool {
        addr >= self.base && addr + len <= self.base + self.size
    }
}

// ---------------------------------------------------------------------------
// Driver descriptor
// ---------------------------------------------------------------------------

/// Maximum MMIO regions a single driver may hold.
const MAX_MMIO_PER_DRIVER: usize = 4;

/// Registration record for one driver.
#[derive(Clone, Copy)]
pub struct DriverInfo {
    /// Human-readable name (e.g. "virtio-net", "uart0").
    pub name: &'static str,
    /// Declared capabilities.
    pub caps: DriverCaps,
    /// Granted MMIO regions.
    pub mmio: [Option<MemRegion>; MAX_MMIO_PER_DRIVER],
    /// If the driver can use interrupts, which line(s).
    pub irq_line: Option<u16>,
    /// State: is the driver currently active?
    pub active: bool,
}

impl DriverInfo {
    pub const fn empty() -> Self {
        Self {
            name: "",
            caps: DriverCaps::none(),
            mmio: [None; MAX_MMIO_PER_DRIVER],
            irq_line: None,
            active: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Driver registry
// ---------------------------------------------------------------------------

/// Maximum number of drivers the kernel can track.
pub const MAX_DRIVERS: usize = 16;

/// Errors returned by the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverError {
    /// The driver table is full.
    RegistryFull,
    /// An MMIO access was outside the driver's granted region.
    MmioOutOfBounds,
    /// The driver tried to use a capability it was not granted.
    CapabilityDenied,
}

/// Central driver registry — tracks all registered drivers and enforces
/// capability checks.
pub struct DriverRegistry {
    drivers: [DriverInfo; MAX_DRIVERS],
    count: usize,
}

impl DriverRegistry {
    pub const fn new() -> Self {
        Self {
            drivers: [DriverInfo::empty(); MAX_DRIVERS],
            count: 0,
        }
    }

    /// Register a new driver.  Returns a driver handle (index).
    pub fn register(&mut self, name: &'static str, caps: DriverCaps) -> Result<usize, DriverError> {
        if self.count >= MAX_DRIVERS {
            return Err(DriverError::RegistryFull);
        }
        let idx = self.count;
        self.drivers[idx] = DriverInfo {
            name,
            caps,
            mmio: [None; MAX_MMIO_PER_DRIVER],
            irq_line: None,
            active: true,
        };
        self.count += 1;
        Ok(idx)
    }

    /// Grant an MMIO region to a registered driver.
    pub fn grant_mmio(&mut self, handle: usize, region: MemRegion) -> Result<(), DriverError> {
        let drv = &mut self.drivers[handle];
        for slot in drv.mmio.iter_mut() {
            if slot.is_none() {
                *slot = Some(region);
                return Ok(());
            }
        }
        Err(DriverError::RegistryFull)
    }

    /// Grant an IRQ line to a registered driver.
    pub fn grant_irq(&mut self, handle: usize, irq: u16) {
        self.drivers[handle].irq_line = Some(irq);
    }

    /// Check whether a driver is allowed to access `addr`.
    pub fn check_mmio(&self, handle: usize, addr: usize) -> Result<(), DriverError> {
        let drv = &self.drivers[handle];
        for region in &drv.mmio {
            if let Some(r) = region {
                if r.contains(addr) {
                    return Ok(());
                }
            }
        }
        Err(DriverError::MmioOutOfBounds)
    }

    /// Check whether a driver is allowed to access `[addr, addr+len)`.
    pub fn check_mmio_range(
        &self,
        handle: usize,
        addr: usize,
        len: usize,
    ) -> Result<(), DriverError> {
        let drv = &self.drivers[handle];
        for region in &drv.mmio {
            if let Some(r) = region {
                if r.contains_range(addr, len) {
                    return Ok(());
                }
            }
        }
        Err(DriverError::MmioOutOfBounds)
    }

    /// Number of registered drivers.
    pub fn count(&self) -> usize {
        self.count
    }

    /// Iterate over active drivers (for diagnostics).
    pub fn iter(&self) -> impl Iterator<Item = &DriverInfo> {
        self.drivers[..self.count].iter().filter(|d| d.active)
    }

    /// Write a summary for the shell `drivers` command.
    pub fn write_list(&self, w: &mut dyn fmt::Write) {
        let _ = fmt::write(
            w,
            format_args!("  ID  NAME              MMIO  IRQ  DMA  NET\n"),
        );
        let _ = fmt::write(
            w,
            format_args!("  --  ----------------  ----  ---  ---  ---\n"),
        );
        for (i, drv) in self.drivers[..self.count].iter().enumerate() {
            if !drv.active {
                continue;
            }
            let mmio_count = drv.mmio.iter().filter(|r| r.is_some()).count();
            let irq_str = match drv.irq_line {
                Some(n) => n as i32,
                None => -1,
            };
            let _ = fmt::write(
                w,
                format_args!(
                    "  {:2}  {:16}  {:4}  {:3}  {:3}  {:3}\n",
                    i,
                    drv.name,
                    mmio_count,
                    irq_str,
                    if drv.caps.uses_dma { "yes" } else { " no" },
                    if drv.caps.uses_network { "yes" } else { " no" },
                ),
            );
        }
    }
}
