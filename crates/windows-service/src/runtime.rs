//! Service state: polling, persistence and policy enforcement.
//!
//! The service is the only component allowed to change firewall state. Every
//! decision is recorded before it becomes visible to the UI, so an interrupted
//! run still has an audit trail.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Local;
use netquota_core::identity::AppIdentity;
use netquota_core::period::window_for;
use netquota_core::policy::{AppPolicy, Enforcement};
use netquota_core::profile::{ProfileKind, VideoQuality};
use netquota_core::quota::{evaluate, Limit, Usage};
use netquota_enforcement::Firewall;
use netquota_monitor::supervisor::Tick;
use netquota_monitor::NetworkMonitor;
use netquota_protocol::wire::{AppUsage, EventRecord, EventKind, GlobalUsage, SiteUsage, Snapshot};
use netquota_protocol::{ErrorCode, MonitoringStatus, Request, Response};
use netquota_quota_engine::QuotaEngine;
use netquota_storage::Store;

pub const SERVICE_VERSION: &str = env!("CARGO_PKG_VERSION");
const PERSIST_INTERVAL_MS: i64 = 5_000;
const EVAL_INTERVAL_MS: i64 = 2_000;
const PRUNE_INTERVAL_MS: i64 = 24 * 60 * 60 * 1000;
const USAGE_RETENTION_DAYS: i64 = 90;

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn hour_bucket(ts: i64) -> i64 {
    ts - ts.rem_euclid(3_600_000)
}

pub fn data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("NETQUOTA_DATA_DIR") {
        return PathBuf::from(dir);
    }
    PathBuf::from(r"C:\ProgramData\NetQuota")
}

pub struct Runtime {
    store: Store,
    monitor: NetworkMonitor,
    firewall: Firewall,
    engine: QuotaEngine,
    policies: HashMap<String, AppPolicy>,
    blocked: HashSet<String>,
    pending_usage: Vec<(String, i64, u64, u64)>,
    apps: Vec<AppUsage>,
    tick: Option<Tick>,
    profile: ProfileKind,
    started_ms: i64,
    last_persist_ms: i64,
    last_eval_ms: i64,
    last_prune_ms: i64,
    data_dir: PathBuf,
    /// Set when ETW is unavailable but interface counters work.
    monitoring: MonitoringStatus,
    unattributed_bytes: u64,
    extension_ids: Vec<String>,
}

impl Runtime {
    pub fn open(data_dir: &Path, dry_run: bool) -> Result<Self> {
        std::fs::create_dir_all(data_dir)
            .with_context(|| format!("cannot create {}", data_dir.display()))?;
        let store = Store::open(&data_dir.join("netquota.db"))?;
        let firewall = Firewall::connect(data_dir.join("firewall-manifest.json"), dry_run)?;
        let mut policies = HashMap::new();
        for policy in store.all_policies()? {
            policies.insert(policy.app_key.clone(), policy);
        }
        let profile = store
            .get_setting("profile")?
            .and_then(|value| serde_json::from_str::<ProfileKind>(&value).ok())
            .unwrap_or_default();
        let extension_ids = read_extension_ids(data_dir);
        if extension_ids.is_empty() {
            tracing::warn!(
                "no extension id allow-list at {}. Chrome requests will be rejected",
                data_dir.join("extension-id.txt").display()
            );
        }

        let monitor = NetworkMonitor::new();
        let monitoring = if monitor.per_process_available() {
            MonitoringStatus::Running
        } else {
            MonitoringStatus::Degraded
        };
        store.add_event(
            now_ms(),
            "service_started",
            "service",
            &format!(
                "monitoring: {}, enforcement: {}",
                match monitoring {
                    MonitoringStatus::Running => "per-process (ETW)",
                    MonitoringStatus::Degraded => "global only (ETW unavailable)",
                    MonitoringStatus::Unavailable => "unavailable",
                },
                if dry_run { "dry-run" } else { "live" }
            ),
            None,
        )?;

        Ok(Self {
            store,
            monitor,
            firewall,
            engine: QuotaEngine::new(),
            policies,
            blocked: HashSet::new(),
            pending_usage: Vec::new(),
            apps: Vec::new(),
            tick: None,
            profile,
            started_ms: now_ms(),
            last_persist_ms: 0,
            last_eval_ms: 0,
            last_prune_ms: now_ms(),
            data_dir: data_dir.to_path_buf(),
            monitoring,
            unattributed_bytes: 0,
            extension_ids,
        })
    }

    /// One poll: attribute traffic, persist deltas, enforce policies.
    pub fn poll(&mut self) {
        let tick = self.monitor.poll();
        let ts = now_ms();
        self.unattributed_bytes = self
            .unattributed_bytes
            .saturating_add(tick.dropped_events);
        if tick.per_process_available {
            self.monitoring = MonitoringStatus::Running;
        } else if self.monitoring != MonitoringStatus::Degraded {
            self.monitoring = MonitoringStatus::Degraded;
        }

        // Register identities and accumulate usage. Nothing is invented when a
        // process cannot be resolved: those apps are reported as Unknown.
        let mut deltas: Vec<(String, u64, u64)> = Vec::new();
        for app in &tick.apps {
            let identity = AppIdentity {
                key: app.app_key.clone(),
                exe_name: app.exe_name.clone(),
                path: app.path.clone(),
                publisher: app.publisher.clone(),
                services: Vec::new(),
            };
            let _ = self.store.upsert_app(&identity, app.category.as_str(), ts);
            if let Some(prev) = self
                .tick
                .as_ref()
                .and_then(|old| old.apps.iter().find(|a| a.app_key == app.app_key))
            {
                let down = app.down.saturating_sub(prev.down);
                let up = app.up.saturating_sub(prev.up);
                if down > 0 || up > 0 {
                    deltas.push((app.app_key.clone(), down, up));
                }
            }
        }

        let hour = hour_bucket(ts);
        for (key, down, up) in deltas {
            self.pending_usage.push((key, hour, down, up));
        }

        if ts - self.last_persist_ms >= PERSIST_INTERVAL_MS {
            self.flush_usage();
            self.last_persist_ms = ts;
        }
        if ts - self.last_eval_ms >= EVAL_INTERVAL_MS {
            self.evaluate_all();
            self.last_eval_ms = ts;
        }
        if ts - self.last_prune_ms >= PRUNE_INTERVAL_MS {
            self.engine.retain(|key| self.policies.contains_key(key));
            let cutoff = ts - USAGE_RETENTION_DAYS * 24 * 3_600_000;
            if let Err(e) = self.store.prune_usage(cutoff) {
                tracing::warn!(error = %e, "usage pruning failed");
            }
            self.last_prune_ms = ts;
        }

        self.tick = Some(tick);
    }

    fn flush_usage(&mut self) {
        if self.pending_usage.is_empty() {
            return;
        }
        let batch = std::mem::take(&mut self.pending_usage);
        if let Err(e) = self.store.add_usage(&batch) {
            tracing::error!(error = %e, entries = batch.len(), "usage write failed; keeping totals in memory");
            // Losing deltas would under-count. Keep them for the next attempt.
            self.pending_usage = batch;
        }
    }

    fn pending_for(&self, app_key: &str, from_ms: i64) -> Usage {
        let mut usage = Usage::default();
        for (key, hour, down, up) in &self.pending_usage {
            if key == app_key && *hour >= from_ms {
                usage.add(*down, *up);
            }
        }
        usage
    }

    fn evaluate_all(&mut self) {
        let Some(tick) = self.tick.clone() else { return };
        let ts = now_ms();
        let today = window_for(netquota_core::period::PeriodKind::Daily, Local::now());
        let month = window_for(netquota_core::period::PeriodKind::Monthly, Local::now());
        let metered = tick.global.metered;
        let wifi = tick
            .global
            .primary_kind
            .map(|k| k == netquota_monitor::global::InterfaceKind::Wifi)
            .unwrap_or(false);

        let mut apps = Vec::with_capacity(tick.apps.len());
        for app in &tick.apps {
            let policy = self.policies.get(&app.app_key).cloned().unwrap_or_else(|| {
                let mut p = AppPolicy::new(&app.app_key);
                p.enforcement = Enforcement::NotifyOnly;
                p
            });
            let window = window_for(policy.period, Local::now());
            let stored = self
                .store
                .usage_for_app(&app.app_key, window.start_ms, window.end_ms)
                .unwrap_or_default();
            let mut used = stored;
            used.add(self.pending_for(&app.app_key, window.start_ms).down, 0);
            used.add(0, self.pending_for(&app.app_key, window.start_ms).up);

            let evaluation = evaluate(used, policy.limit, policy.credit.as_ref(), ts, 0.8);

            let background_blocked = app.background
                && matches!(policy.enforcement, Enforcement::BlockBackground)
                && (
                    policy.background.block_in_background
                        || (policy.background.foreground_only && !app.background)
                        || (policy.background.block_on_metered && metered)
                        || (policy.background.wifi_only && !wifi)
                );

            let decision = self.engine.decide(
                &app.app_key,
                window.start_ms,
                &evaluation,
                policy.enforcement,
                background_blocked,
                policy.paused,
            );

            if decision.transition == netquota_quota_engine::Transition::Exhausted && decision.notify {
                let _ = self.store.add_event(
                    ts,
                    "quota_exhausted",
                    &app.app_key,
                    &format!(
                        "{} reached its {} limit ({})",
                        app.exe_name,
                        policy.period.as_str(),
                        netquota_core::units::format_bytes(used.total())
                    ),
                    Some(used.total()),
                );
            }
            if decision.transition == netquota_quota_engine::Transition::Recovered {
                let _ = self.store.add_event(
                    ts,
                    "info",
                    &app.app_key,
                    &format!("{} is within its quota again", app.exe_name),
                    None,
                );
            }

            self.apply_block(&app.app_key, app.path.as_deref(), decision.block, ts);

            let daily = self.usage_window(&app.app_key, today.start_ms, today.end_ms);
            let monthly = self.usage_window(&app.app_key, month.start_ms, month.end_ms);

            apps.push(AppUsage {
                identity: AppIdentity {
                    key: app.app_key.clone(),
                    exe_name: app.exe_name.clone(),
                    path: app.path.clone(),
                    publisher: app.publisher.clone(),
                    services: Vec::new(),
                },
                pid: app.pid,
                category: app.category.as_str().to_string(),
                down: app.down,
                up: app.up,
                speed_down_bps: app.down_speed as f64,
                speed_up_bps: app.up_speed as f64,
                daily,
                monthly,
                period_used: used,
                limit: policy.limit,
                remaining_total: evaluation.remaining_total,
                exhausted: evaluation.exhausted.is_some(),
                paused: policy.paused,
                blocked: self.blocked.contains(&app.app_key),
                background: app.background,
                confidence: app.confidence,
                protection_note: policy.protection_note,
                last_seen_ms: ts,
                has_icon: self.store.list_apps().ok().is_some_and(|list| {
                    list.iter().any(|a| a.identity.key == app.app_key && a.icon_path.is_some())
                }),
            });
        }
        self.apps = apps;
    }

    fn usage_window(&self, app_key: &str, from: i64, to: i64) -> Usage {
        let mut usage = self
            .store
            .usage_for_app(app_key, from, to)
            .unwrap_or_default();
        usage.add(self.pending_for(app_key, from).down, self.pending_for(app_key, from).up);
        usage
    }

    fn apply_block(&mut self, app_key: &str, path: Option<&str>, should_block: bool, ts: i64) {
        let is_blocked = self.blocked.contains(app_key);
        if should_block == is_blocked {
            return;
        }
        if should_block {
            let Some(path) = path else {
                tracing::warn!(app_key, "cannot block: executable path is unknown");
                let _ = self.store.add_event(
                    ts,
                    "warning",
                    app_key,
                    "Cannot restrict this process: its executable path is unavailable",
                    None,
                );
                return;
            };
            match self.firewall.block(Path::new(path)) {
                Ok(rule) => {
                    self.blocked.insert(app_key.to_string());
                    let _ = self.store.add_event(
                        ts,
                        "blocked",
                        app_key,
                        &format!("Network access restricted (rule {rule})"),
                        None,
                    );
                }
                Err(e) => {
                    tracing::error!(app_key, error = %e, "firewall block failed");
                    let _ = self.store.add_event(
                        ts,
                        "warning",
                        app_key,
                        &format!("Could not apply the restriction: {e}"),
                        None,
                    );
                }
            }
        } else {
            match self.firewall.unblock(Path::new(path.unwrap_or_default())) {
                Ok(_) => {
                    self.blocked.remove(app_key);
                    let _ = self.store.add_event(
                        ts,
                        "unblocked",
                        app_key,
                        "Network access restored",
                        None,
                    );
                }
                Err(e) => {
                    tracing::error!(app_key, error = %e, "firewall unblock failed");
                    let _ = self.store.add_event(
                        ts,
                        "warning",
                        app_key,
                        &format!("Could not remove the restriction: {e}"),
                        None,
                    );
                }
            }
        }
    }

    fn global_usage(&self) -> GlobalUsage {
        let today = window_for(netquota_core::period::PeriodKind::Daily, Local::now());
        let totals = self
            .store
            .usage_totals(today.start_ms, today.end_ms)
            .unwrap_or_default();
        let tick = self.tick.as_ref();
        GlobalUsage {
            down: tick.map(|t| t.global.down_total).unwrap_or(0),
            up: tick.map(|t| t.global.up_total).unwrap_or(0),
            today_down: totals.down,
            today_up: totals.up,
            down_bps: tick.map(|t| t.down_speed as f64).unwrap_or(0.0),
            up_bps: tick.map(|t| t.up_speed as f64).unwrap_or(0.0),
        }
    }

    fn sites(&self) -> Vec<SiteUsage> {
        let now = now_ms();
        let stored = self.store.list_sites().unwrap_or_default();
        stored
            .into_iter()
            .map(|site| {
                let (limit_bytes, blocked) =
                    self.store.site_limit(&site.host).ok().flatten().unwrap_or((None, false));
                let limit = Limit {
                    total: limit_bytes,
                    ..Limit::UNLIMITED
                };
                let used = Usage {
                    down: site.down,
                    up: site.up,
                };
                let evaluation = evaluate(used, limit, None, now, 0.8);
                let exhausted = evaluation.exhausted.is_some();
                SiteUsage {
                    host: site.host,
                    down: site.down,
                    up: site.up,
                    estimated: true,
                    active_secs: site.active_secs,
                    tabs: 0,
                    limit,
                    remaining_total: evaluation.remaining_total,
                    exhausted,
                    blocked: blocked || (exhausted && blocked),
                }
            })
            .collect()
    }

    fn events(&self, limit: u32) -> Vec<EventRecord> {
        self.store
            .events(limit)
            .unwrap_or_default()
            .into_iter()
            .map(|event| EventRecord {
                id: event.id,
                ts_ms: event.ts_ms,
                kind: event_kind(&event.kind),
                subject: event.subject,
                message: event.message,
                bytes: event.bytes,
            })
            .collect()
    }

    pub fn snapshot(&self) -> Snapshot {
        let tick = self.tick.as_ref();
        Snapshot {
            generated_at_ms: now_ms(),
            service_version: SERVICE_VERSION.to_string(),
            monitoring: self.monitoring,
            enforcement_ready: self.firewall.is_live(),
            global: self.global_usage(),
            apps: self.apps.clone(),
            sites: self.sites(),
            profile: self.profile.as_str().to_string(),
            profile_video_height: VideoQuality::for_profile(self.profile).target_height(),
            metered: tick.map(|t| t.global.metered).unwrap_or(false),
            interface: tick.and_then(|t| t.global.primary_alias.clone()),
            events: self.events(20),
            unattributed_bytes: self.unattributed_bytes,
        }
    }

    pub fn apps(&self) -> Vec<AppUsage> {
        self.apps.clone()
    }

    pub fn handle(&mut self, request: Request) -> Response {
        match request {
            Request::Hello { client, extension_id, .. } => {
                if client == netquota_protocol::ClientKind::ChromeExtension {
                    let ok = extension_id
                        .as_deref()
                        .is_some_and(|id| self.extension_ids.iter().any(|known| known == id));
                    if !ok {
                        return Response::Error {
                            code: ErrorCode::Unauthorized,
                            message: "extension id is not allow-listed".into(),
                        };
                    }
                }
                Response::Hello {
                    version: netquota_protocol::PROTOCOL_VERSION,
                    service_version: SERVICE_VERSION.to_string(),
                    enforcement_ready: self.firewall.is_live(),
                    monitoring: self.monitoring,
                }
            }
            Request::Ping => Response::Pong,
            Request::Snapshot => Response::Snapshot { snapshot: self.snapshot() },
            Request::ListApps => Response::Apps { apps: self.apps() },
            Request::SetPolicy { policy } => match self.store.put_policy(&policy, now_ms()) {
                Ok(()) => {
                    self.policies.insert(policy.app_key.clone(), policy);
                    Response::PolicyOk
                }
                Err(e) => Response::Error {
                    code: ErrorCode::Internal,
                    message: e.to_string(),
                },
            },
            Request::GrantCredit {
                app_key,
                extra_bytes,
                duration_secs,
            } => {
                let Some(mut policy) = self.policies.get(&app_key).cloned().or_else(|| {
                    self.store.get_policy(&app_key).ok().flatten()
                }) else {
                    return Response::Error {
                        code: ErrorCode::NotFound,
                        message: "set a limit for this application first".into(),
                    };
                };
                let credit = policy.credit.get_or_insert(netquota_core::quota::Credit {
                    extra_total: 0,
                    extra_down: 0,
                    extra_up: 0,
                    until_ms: None,
                    reason: String::new(),
                });
                credit.extra_total = credit.extra_total.saturating_add(extra_bytes);
                credit.reason = "user top-up".into();
                if let Some(secs) = duration_secs {
                    credit.until_ms = Some(now_ms() + (secs as i64) * 1000);
                }
                if let Err(e) = self.store.put_policy(&policy, now_ms()) {
                    return Response::Error {
                        code: ErrorCode::Internal,
                        message: e.to_string(),
                    };
                }
                self.policies.insert(app_key.clone(), policy);
                let _ = self.store.add_event(
                    now_ms(),
                    "credit_granted",
                    &app_key,
                    &format!("Added {}", netquota_core::units::format_bytes(extra_bytes)),
                    Some(extra_bytes),
                );
                Response::PolicyOk
            }
            Request::SetPaused { app_key, paused } => {
                let mut policy = self.policies.get(&app_key).cloned().unwrap_or_else(|| {
                    AppPolicy::new(&app_key)
                });
                policy.paused = paused;
                let _ = self.store.put_policy(&policy, now_ms());
                self.policies.insert(app_key, policy);
                Response::PolicyOk
            }
            Request::BlockNow { app_key } => {
                let path = self
                    .store
                    .list_apps()
                    .unwrap_or_default()
                    .into_iter()
                    .find(|a| a.identity.key == app_key)
                    .and_then(|a| a.identity.path);
                match path {
                    Some(path) => match self.firewall.block(Path::new(&path)) {
                        Ok(_) => {
                            self.blocked.insert(app_key.clone());
                            let _ = self.store.add_event(
                                now_ms(),
                                "blocked",
                                &app_key,
                                "Blocked on user request",
                                None,
                            );
                            Response::PolicyOk
                        }
                        Err(e) => Response::Error {
                            code: ErrorCode::Internal,
                            message: e.to_string(),
                        },
                    },
                    None => Response::Error {
                        code: ErrorCode::NotFound,
                        message: "application path unknown".into(),
                    },
                }
            }
            Request::Unblock { app_key } => {
                let path = self
                    .store
                    .list_apps()
                    .unwrap_or_default()
                    .into_iter()
                    .find(|a| a.identity.key == app_key)
                    .and_then(|a| a.identity.path);
                if let Some(path) = path {
                    let _ = self.firewall.unblock(Path::new(&path));
                }
                self.blocked.remove(&app_key);
                let _ = self.store.add_event(
                    now_ms(),
                    "unblocked",
                    &app_key,
                    "Restored on user request",
                    None,
                );
                Response::PolicyOk
            }
            Request::ApplyProfile { profile } => {
                if let Err(e) = self.store.set_setting("profile", &serde_json::to_string(&profile).unwrap_or_default()) {
                    return Response::Error {
                        code: ErrorCode::Internal,
                        message: e.to_string(),
                    };
                }
                self.profile = profile;
                Response::PolicyOk
            }
            Request::SiteUsage { site, down, up, active_secs } => {
                if let Err(e) = self.store.add_site_usage(&site, down, up, active_secs, now_ms()) {
                    return Response::Error {
                        code: ErrorCode::Internal,
                        message: e.to_string(),
                    };
                }
                Response::PolicyOk
            }
            Request::ListSites => Response::Sites { sites: self.sites() },
            Request::SetSitePolicy { site, limit_bytes, blocked } => {
                match self.store.set_site_policy(&site, limit_bytes, blocked, now_ms()) {
                    Ok(()) => Response::PolicyOk,
                    Err(e) => Response::Error {
                        code: ErrorCode::Internal,
                        message: e.to_string(),
                    },
                }
            }
            Request::ListEvents { limit } => Response::Events { events: self.events(limit) },
            Request::EmergencyRestore => match self.firewall.restore_all() {
                Ok(removed) => {
                    self.blocked.clear();
                    for app in self.policies.values() {
                        self.engine.forget(&app.app_key);
                    }
                    let _ = self.store.add_event(
                        now_ms(),
                        "recovery",
                        "service",
                        &format!("Removed {removed} NetQuota firewall rule(s)"),
                        None,
                    );
                    Response::Restored {
                        removed_rules: removed as u32,
                    }
                }
                Err(e) => Response::Error {
                    code: ErrorCode::Internal,
                    message: e.to_string(),
                },
            },
            Request::Shutdown => Response::Error {
                code: ErrorCode::Unsupported,
                message: "stop the service with Services or: netquota-cli recover".into(),
            },
        }
    }

    /// Remove every rule NetQuota owns. Called on stop so a crash or a stopped
    /// service never leaves the machine restricted.
    pub fn shutdown(&mut self) {
        self.flush_usage();
        match self.firewall.restore_all() {
            Ok(removed) if removed > 0 => tracing::info!(removed, "restored network access"),
            Ok(_) => {}
            Err(e) => tracing::error!(error = %e, "could not restore firewall rules"),
        }
        let _ = self.store.add_event(
            now_ms(),
            "service_stopped",
            "service",
            "Monitoring stopped; NetQuota rules removed",
            None,
        );
        let _ = self.store.backup_to(&self.data_dir.join("netquota.db.bak"));
        tracing::info!(uptime_ms = now_ms() - self.started_ms, "service stopped");
    }

}

fn event_kind(value: &str) -> EventKind {
    match value {
        "quota_exhausted" => EventKind::QuotaExhausted,
        "blocked" => EventKind::Blocked,
        "unblocked" => EventKind::Unblocked,
        "credit_granted" => EventKind::CreditGranted,
        "warning" => EventKind::Warning,
        "service_started" => EventKind::ServiceStarted,
        "service_stopped" => EventKind::ServiceStopped,
        "recovery" => EventKind::Recovery,
        _ => EventKind::Info,
    }
}

/// Allow-list of Chrome extension ids, one per line. Written by the installer.
fn read_extension_ids(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("extension-id.txt"))
        .map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && line.len() <= 64)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> (Runtime, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "netquota-service-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let runtime = Runtime::open(&dir, true).expect("runtime opens in dry-run mode");
        (runtime, dir)
    }

    #[test]
    fn hello_rejects_unlisted_extension() {
        let (mut rt, dir) = runtime();
        let response = rt.handle(Request::Hello {
            version: netquota_protocol::PROTOCOL_VERSION,
            client: netquota_protocol::ClientKind::ChromeExtension,
            extension_id: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()),
        });
        assert!(matches!(
            response,
            Response::Error { code: ErrorCode::Unauthorized, .. }
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn hello_accepts_desktop_client() {
        let (mut rt, dir) = runtime();
        let response = rt.handle(Request::Hello {
            version: netquota_protocol::PROTOCOL_VERSION,
            client: netquota_protocol::ClientKind::Desktop,
            extension_id: None,
        });
        assert!(matches!(response, Response::Hello { .. }));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn policy_is_persisted_and_visible() {
        let (mut rt, dir) = runtime();
        let mut policy = AppPolicy::new("c:\\test\\app.exe");
        policy.limit = Limit { total: Some(1_000_000), download: None, upload: None };
        assert!(matches!(
            rt.handle(Request::SetPolicy { policy: policy.clone() }),
            Response::PolicyOk
        ));
        assert_eq!(rt.store.get_policy(&policy.app_key).unwrap(), Some(policy));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn credit_without_policy_is_rejected() {
        let (mut rt, dir) = runtime();
        let response = rt.handle(Request::GrantCredit {
            app_key: "c:\\nope.exe".into(),
            extra_bytes: 50_000_000,
            duration_secs: None,
        });
        assert!(matches!(response, Response::Error { code: ErrorCode::NotFound, .. }));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn emergency_restore_is_always_available() {
        let (mut rt, dir) = runtime();
        assert!(matches!(
            rt.handle(Request::EmergencyRestore),
            Response::Restored { .. }
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn shutdown_leaves_an_event() {
        let (mut rt, dir) = runtime();
        rt.shutdown();
        let events = rt.events(10);
        assert!(events.iter().any(|e| e.kind == EventKind::ServiceStopped));
        let _ = std::fs::remove_dir_all(dir);
    }
}



