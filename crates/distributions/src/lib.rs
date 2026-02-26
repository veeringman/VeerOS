#![no_std]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Distribution {
    Minimal,
    Application,
    RealTime,
    Full,
}

pub const fn active_distribution() -> Distribution {
    #[cfg(feature = "full")]
    {
        return Distribution::Full;
    }

    #[cfg(all(not(feature = "full"), feature = "real-time"))]
    {
        return Distribution::RealTime;
    }

    #[cfg(all(not(feature = "full"), not(feature = "real-time"), feature = "app"))]
    {
        return Distribution::Application;
    }

    Distribution::Minimal
}