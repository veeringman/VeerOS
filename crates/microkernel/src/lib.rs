#![no_std]

pub mod ipc;
pub mod task;

use arch::Platform;
use bitflags::bitflags;

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