//! Data shapes shared by the service, the desktop app and the CLI. They are
//! intentionally explicit instead of mirroring internal state, so the service
//! controls exactly what leaves the process.

use netquota_core::identity::AppIdentity;
use netquota_core::quota::{Limit, Usage};
use serde::{Deserialize, Serialize};

use crate::MonitoringStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Counted from kernel events for this process.
    Measured,
    /// Partially attributed (e.g. shared host, proxy or VPN traffic).
    Estimated,
    /// Traffic exists but cannot be attributed to a process.
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GlobalUsage {
    pub down: u64,
    pub up: u64,
    pub today_down: u64,
    pub today_up: u64,
    pub down_bps: f64,
    pub up_bps: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppUsage {
    pub identity: AppIdentity,
    pub pid: u32,
    pub category: String,
    pub down: u64,
    pub up: u64,
    pub speed_down_bps: f64,
    pub speed_up_bps: f64,
    pub daily: Usage,
    pub monthly: Usage,
    pub period_used: Usage,
    pub limit: Limit,
    pub remaining_total: Option<i64>,
    pub exhausted: bool,
    pub paused: bool,
    pub blocked: bool,
    pub background: bool,
    pub confidence: Confidence,
    pub protection_note: Option<String>,
    pub last_seen_ms: i64,
    pub has_icon: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SiteUsage {
    pub host: String,
    pub down: u64,
    pub up: u64,
    pub estimated: bool,
    pub active_secs: u64,
    pub tabs: u32,
    pub limit: Limit,
    pub remaining_total: Option<i64>,
    pub exhausted: bool,
    pub blocked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    QuotaExhausted,
    Blocked,
    Unblocked,
    CreditGranted,
    Warning,
    ServiceStarted,
    ServiceStopped,
    Recovery,
    Info,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventRecord {
    pub id: i64,
    pub ts_ms: i64,
    pub kind: EventKind,
    pub subject: String,
    pub message: String,
    pub bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub generated_at_ms: i64,
    pub service_version: String,
    pub monitoring: MonitoringStatus,
    pub enforcement_ready: bool,
    pub global: GlobalUsage,
    pub apps: Vec<AppUsage>,
    pub sites: Vec<SiteUsage>,
    pub profile: String,
    pub profile_video_height: Option<u32>,
    pub metered: bool,
    pub interface: Option<String>,
    pub events: Vec<EventRecord>,
    pub unattributed_bytes: u64,
}

impl Snapshot {
    pub fn empty(service_version: &str, monitoring: MonitoringStatus) -> Self {
        Self {
            generated_at_ms: chrono::Utc::now().timestamp_millis(),
            service_version: service_version.to_string(),
            monitoring,
            enforcement_ready: false,
            global: GlobalUsage::default(),
            apps: Vec::new(),
            sites: Vec::new(),
            profile: "normal".into(),
            profile_video_height: None,
            metered: false,
            interface: None,
            events: Vec::new(),
            unattributed_bytes: 0,
        }
    }
}
