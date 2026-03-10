//! VeerOS distribution profile definitions.
//!
//! # Distribution Matrix
//!
//! VeerOS uses two orthogonal axes to determine what gets bundled:
//!
//! ## Axis 1 — Distribution Profile (scheduler + capabilities)
//!
//! | Profile        | Scheduler   | Caps enabled                                   |
//! |---------------|-------------|------------------------------------------------|
//! | `dist-minimal` | round-robin | IPC, VIRTUAL_MEMORY, DRIVER_ISOLATION           |
//! | `dist-app`     | application | + NETWORK_STACK, APPLICATION_RUNTIME            |
//! | `dist-rt`      | priority    | + REAL_TIME_SCHEDULER                           |
//! | `dist-full`    | priority    | all of the above                                |
//!
//! ## Axis 2 — Optional Components (toggled independently or via profile)
//!
//! | Component    | `dist-minimal` | `dist-app` | `dist-rt` | `dist-full` |
//! |-------------|:--------------:|:----------:|:---------:|:-----------:|
//! | shell       |       —        |     ✓      |     —     |      ✓      |
//! | net         |       —        |     ✓      |     —     |      ✓      |
//! | userlib     |       —        |     ✓      |     —     |      ✓      |
//! | samples     |       —        |     ✓      |     —     |      ✓      |
//! | wifi        |       —        |    opt     |     —     |     opt     |
//! | ble         |       —        |    opt     |     —     |     opt     |
//! | ieee802154  |       —        |    opt     |     —     |     opt     |
//!
//! *opt = available only on boards with the hardware; not auto-enabled.*
//!
//! Components can also be toggled independently of the profile.
//! For example: `--features dist-minimal,shell` gives a minimal kernel
//! with just the interactive shell.
//!
//! ## Build examples
//!
//! ```sh
//! # Minimal kernel (bare scheduler, no shell/net):
//! cargo build -p kernel-qemu-virt --no-default-features --features dist-minimal
//!
//! # App kernel (shell + net + userlib + samples):
//! cargo build -p kernel-qemu-virt --features dist-app
//!
//! # Minimal + shell only:
//! cargo build -p kernel-qemu-virt --no-default-features --features dist-minimal,shell
//!
//! # Full ESP32 with all radios:
//! cargo build -p kernel-xiao-esp32c6 --features dist-full,wifi,ble,ieee802154
//! ```

#![no_std]

/// Distribution profile identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Distribution {
    Minimal,
    Application,
    RealTime,
    Full,
}

/// Returns the active distribution profile based on compile-time features.
pub const fn active_distribution() -> Distribution {
    #[cfg(feature = "dist-full")]
    {
        return Distribution::Full;
    }

    #[cfg(all(not(feature = "dist-full"), feature = "dist-rt"))]
    {
        return Distribution::RealTime;
    }

    #[cfg(all(not(feature = "dist-full"), not(feature = "dist-rt"), feature = "dist-app"))]
    {
        return Distribution::Application;
    }

    #[allow(unreachable_code)]
    Distribution::Minimal
}