use serde::{Deserialize, Serialize};

use crate::period::PeriodKind;
use crate::profile::ProfileKind;
use crate::quota::{Credit, Limit};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Enforcement {
    /// Block network access when the quota is exhausted.
    #[default]
    Block,
    /// Record and notify only; used while the user evaluates a policy.
    NotifyOnly,
    /// Only background traffic is blocked on exhaustion.
    BlockBackground,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackgroundPolicy {
    pub block_in_background: bool,
    pub foreground_only: bool,
    pub block_on_metered: bool,
    pub wifi_only: bool,
    /// Allowed background window in local hours `[start, end)`; `None` = any time.
    pub allowed_hours: Option<(u8, u8)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppPolicy {
    pub app_key: String,
    #[serde(default)]
    pub limit: Limit,
    #[serde(default)]
    pub period: PeriodKind,
    #[serde(default)]
    pub enforcement: Enforcement,
    #[serde(default)]
    pub background: BackgroundPolicy,
    /// Outbound-only cap, in KB/s. Enforced by the QoS provider when available.
    #[serde(default)]
    pub upload_kbps: Option<u32>,
    /// Requested inbound cap. Not enforceable by the firewall or QoS providers;
    /// surfaced in the UI as unsupported until a shaping provider exists.
    #[serde(default)]
    pub download_kbps: Option<u32>,
    #[serde(default)]
    pub paused: bool,
    /// Which saver profile is currently applied to this app.
    #[serde(default)]
    pub profile: ProfileKind,
    #[serde(default)]
    pub credit: Option<Credit>,
    /// Set by the engine when a protected process must not be blocked.
    #[serde(default)]
    pub protection_note: Option<String>,
}

impl AppPolicy {
    pub fn new(app_key: impl Into<String>) -> Self {
        Self {
            app_key: app_key.into(),
            limit: Limit::default(),
            period: PeriodKind::default(),
            enforcement: Enforcement::default(),
            background: BackgroundPolicy::default(),
            upload_kbps: None,
            download_kbps: None,
            paused: false,
            profile: ProfileKind::default(),
            credit: None,
            protection_note: None,
        }
    }

    /// Trust-boundary validation for policies received over IPC. Rejects values
    /// that could overflow arithmetic or turn into absurd firewall/QoS input.
    pub fn validate(&self) -> Result<(), String> {
        const MAX_BYTES: u64 = 100_000_000_000_000; // 100 TB
        if self.app_key.is_empty() || self.app_key.len() > 1_024 {
            return Err("invalid app key".into());
        }
        for value in [self.limit.total, self.limit.download, self.limit.upload]
            .into_iter()
            .flatten()
        {
            if value > MAX_BYTES {
                return Err("limit out of range".into());
            }
        }
        if let PeriodKind::CustomDays { days } = self.period {
            if days == 0 || days > 366 {
                return Err("custom period must be 1..=366 days".into());
            }
        }
        if let Some(hours) = self.background.allowed_hours {
            if hours.0 > 23 || hours.1 > 24 {
                return Err("allowed_hours out of range".into());
            }
        }
        for kbps in [self.upload_kbps, self.download_kbps].into_iter().flatten() {
            if kbps == 0 || kbps > 4_000_000 {
                return Err("speed cap out of range".into());
            }
        }
        if let Some(credit) = &self.credit {
            let sum = credit
                .extra_total
                .saturating_add(credit.extra_down)
                .saturating_add(credit.extra_up);
            if sum > MAX_BYTES {
                return Err("credit out of range".into());
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_policy_is_unlimited_and_allowed() {
        let p = AppPolicy::new("c:\\app.exe");
        assert!(p.limit.is_unlimited());
        assert_eq!(p.enforcement, Enforcement::Block);
        assert!(!p.paused);
    }

    #[test]
    fn policies_round_trip_through_json_with_defaults() {
        let json = r#"{"app_key":"x","limit":{"total":1000,"download":null,"upload":null}}"#;
        let p: AppPolicy = serde_json::from_str(json).expect("deserialises with defaults");
        assert_eq!(p.limit.total, Some(1000));
        assert_eq!(p.period, PeriodKind::Daily);
    }
}
