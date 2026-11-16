//! Applies and removes network restrictions.
//!
//! Ownership is the core safety property: every rule NetQuota creates carries
//! the [`GROUP`] grouping string, and only rules with that exact group are ever
//! modified or deleted. Existing user and system firewall rules are untouched.

#[cfg(windows)]
mod firewall;

#[cfg(windows)]
pub use firewall::{BlockState, Firewall, OwnedRule, GROUP, RULE_PREFIX};

/// Speed limits are stored in policies but not enforced yet. Windows QoS
/// (`ThrottleRateActionBitsPerSecond`) shapes outbound traffic only and is not
/// a general shaper, so claiming bidirectional shaping would be false.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeedLimitSupport {
    /// Upload throttling through a Windows QoS policy (outbound only).
    UploadExperimental,
    /// Download and combined limits need a WFP callout driver.
    UnsupportedDownload,
}
