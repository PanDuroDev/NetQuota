//! Windows network monitoring. Platform-specific code is isolated here so the
//! quota engine never depends on Win32 details.

pub mod supervisor;

#[cfg(windows)]
pub mod etw;
#[cfg(windows)]
pub mod global;
#[cfg(windows)]
pub mod icons;
#[cfg(windows)]
pub mod metered;
#[cfg(windows)]
pub mod process;
#[cfg(windows)]
pub mod services;

pub use supervisor::{AppRuntime, GlobalRuntime, MonitorState, NetworkMonitor};
