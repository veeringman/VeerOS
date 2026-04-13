#![no_std]

pub mod alloc;
pub mod accelerator;
pub mod audit;
pub mod channel;
pub mod dispatch;
pub mod driver;
pub mod fat32;
pub mod futex;
pub mod hid;
pub mod input;
pub mod ipc;
pub mod klog;
pub mod poll;
pub mod process;
pub mod ramfs;
pub mod socket;
pub mod syscall;
pub mod task;
pub mod user;
pub mod vfs;

use arch::Platform;
use bitflags::bitflags;

// ═══════════════════════════════════════════════════════════════════════════
// System-wide CSPRNG
// ═══════════════════════════════════════════════════════════════════════════

/// Global system CSPRNG (ChaCha20-DRBG).
///
/// **Must** be seeded from hardware entropy at boot via [`seed_system_rng`]
/// before `/dev/random` is usable.
static mut SYSTEM_RNG: Option<crypto::rng::ChaChaRng> = None;

/// Seed the system CSPRNG with 32 bytes of hardware entropy.
///
/// Called once during kernel init (e.g. from ESP32 RNG peripheral,
/// BCM2712 TRNG, RDRAND, etc). Safe to call again to reseed.
///
/// # Safety
///
/// Must be called from single-threaded kernel init context. The microkernel
/// is single-core / cooperative, so no concurrent access after boot.
pub unsafe fn seed_system_rng(seed: [u8; 32]) {
    let rng_ptr = core::ptr::addr_of_mut!(SYSTEM_RNG);
    rng_ptr.write(Some(crypto::rng::ChaChaRng::from_seed(seed)));
}

/// Fill `buf` with cryptographically secure random bytes from the system RNG.
///
/// Returns the number of bytes written (0 if RNG not yet seeded).
pub fn system_rng_fill(buf: &mut [u8]) -> usize {
    unsafe {
        let rng_ptr = core::ptr::addr_of_mut!(SYSTEM_RNG);
        if let Some(rng) = (*rng_ptr).as_mut() {
            use crypto::CryptoRng;
            rng.fill_bytes(buf);
            buf.len()
        } else {
            0
        }
    }
}

bitflags! {
    pub struct CapabilitySet: u32 {
        const IPC = 1 << 0;
        const VIRTUAL_MEMORY = 1 << 1;
        const DRIVER_ISOLATION = 1 << 2;
        const NETWORK_STACK = 1 << 3;
        const APPLICATION_RUNTIME = 1 << 4;
        const REAL_TIME_SCHEDULER = 1 << 5;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerProfile {
    Minimal,
    Application,
    RealTime,
}

pub struct KernelConfig {
    pub scheduler: SchedulerProfile,
    pub capabilities: CapabilitySet,
}

impl KernelConfig {
    pub const fn from_features() -> Self {
        #[allow(unused_mut)]
        let mut capabilities = CapabilitySet::IPC
            .union(CapabilitySet::VIRTUAL_MEMORY)
            .union(CapabilitySet::DRIVER_ISOLATION);

        #[cfg(feature = "dist-app")]
        {
            capabilities = capabilities
                .union(CapabilitySet::NETWORK_STACK)
                .union(CapabilitySet::APPLICATION_RUNTIME);
        }

        #[cfg(feature = "dist-rt")]
        {
            capabilities = capabilities.union(CapabilitySet::REAL_TIME_SCHEDULER);
        }

        let scheduler = {
            #[cfg(feature = "dist-rt")]
            {
                SchedulerProfile::RealTime
            }
            #[cfg(all(not(feature = "dist-rt"), feature = "dist-app"))]
            {
                SchedulerProfile::Application
            }
            #[cfg(all(not(feature = "dist-rt"), not(feature = "dist-app")))]
            {
                SchedulerProfile::Minimal
            }
        };

        Self {
            scheduler,
            capabilities,
        }
    }
}

pub struct Kernel<P: Platform> {
    pub config: KernelConfig,
    platform: P,
}

impl<P: Platform> Kernel<P> {
    pub fn new(platform: P) -> Self {
        Self {
            config: KernelConfig::from_features(),
            platform,
        }
    }

    /// Perform early platform init (CPU, interrupts, timer).
    pub fn boot(&self) {
        self.platform.init_cpu();
        self.platform.init_interrupts();
        self.platform.init_timer();
    }

    /// Return platform name for banner / diagnostics.
    pub fn platform_name(&self) -> &'static str {
        self.platform.name()
    }

    /// Return scheduler profile label.
    pub fn scheduler_label(&self) -> &'static str {
        match self.config.scheduler {
            SchedulerProfile::Minimal => "minimal",
            SchedulerProfile::Application => "application",
            SchedulerProfile::RealTime => "real-time",
        }
    }
}