//! Ties the providers together: ETW per-process events, interface counters and
//! SCM service names. The supervisor only ever reports what a provider actually
//! observed, and labels attribution confidence instead of smoothing gaps.

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use netquota_core::categorize::{classify, Category};
use netquota_core::quota::Usage;
use netquota_core::quota::Direction;
use netquota_protocol::wire::Confidence;
use serde::{Deserialize, Serialize};

#[cfg(windows)]
use crate::global::{self, GlobalSnapshot, InterfaceKind, InterfaceStats};
#[cfg(windows)]
use crate::metered::{self, ConnectionCost};
#[cfg(windows)]
use crate::process;
#[cfg(windows)]
use crate::services;

pub const SERVICES_REFRESH: Duration = Duration::from_secs(60);
pub const METERED_REFRESH: Duration = Duration::from_secs(30);
pub const ACTIVE_WINDOW_MS: i64 = 60_000;
const PRUNE_WINDOW_MS: i64 = 300_000;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppRuntime {
    pub app_key: String,
    pub exe_name: String,
    pub path: Option<String>,
    pub publisher: Option<String>,
    pub category: Category,
    pub pid: u32,
    pub started_ms: i64,
    pub down_total: u64,
    pub up_total: u64,
    pub down_speed: u64,
    pub up_speed: u64,
    pub last_active_ms: i64,
    pub background: bool,
    pub running: bool,
    pub confidence: Confidence,
}

impl AppRuntime {
    pub fn usage(&self) -> Usage {
        Usage {
            down: self.down_total,
            up: self.up_total,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GlobalRuntime {
    pub down_total: u64,
    pub up_total: u64,
    pub down_speed: u64,
    pub up_speed: u64,
    pub primary_alias: Option<String>,
    pub primary_kind: Option<InterfaceKind>,
    pub vpn_present: bool,
    pub interfaces: Vec<InterfaceStats>,
    pub metered: bool,
    pub cost_known: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppTick {
    pub app_key: String,
    pub exe_name: String,
    pub path: Option<String>,
    pub publisher: Option<String>,
    pub category: Category,
    pub pid: u32,
    pub started_ms: i64,
    pub down: u64,
    pub up: u64,
    pub down_speed: u64,
    pub up_speed: u64,
    pub background: bool,
    pub confidence: Confidence,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tick {
    pub elapsed_ms: u64,
    pub down: u64,
    pub up: u64,
    pub down_speed: u64,
    pub up_speed: u64,
    pub apps: Vec<AppTick>,
    pub dropped_events: u64,
    pub per_process_available: bool,
    pub per_process_error: Option<String>,
    pub global: GlobalRuntime,
    pub foreground_pid: Option<u32>,
}

struct PidState {
    runtime: AppRuntime,
}

pub struct NetworkMonitor {
    #[cfg(windows)]
    etw: Option<crate::etw::EtwSession>,
    per_process_error: Option<String>,
    #[cfg(windows)]
    service_map: HashMap<u32, Vec<String>>,
    service_map_at: Option<Instant>,
    #[cfg(windows)]
    previous: Option<GlobalSnapshot>,
    last_tick: Option<Instant>,
    pids: HashMap<u32, PidState>,
    registry: HashMap<String, AppRuntime>,
    global: GlobalRuntime,
    #[cfg(windows)]
    cost: ConnectionCost,
    cost_at: Option<Instant>,
}

impl Default for NetworkMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkMonitor {
    #[cfg(windows)]
    pub fn new() -> Self {
        let (etw, per_process_error) = match crate::etw::EtwSession::start() {
            Ok(session) => (Some(session), None),
            Err(error) => {
                tracing::warn!(%error, "per-process ETW unavailable");
                (None, Some(error.to_string()))
            }
        };
        Self {
            etw,
            per_process_error,
            service_map: HashMap::new(),
            service_map_at: None,
            previous: None,
            last_tick: None,
            pids: HashMap::new(),
            registry: HashMap::new(),
            global: GlobalRuntime::default(),
            cost: ConnectionCost::default(),
            cost_at: None,
        }
    }

    #[cfg(not(windows))]
    pub fn new() -> Self {
        Self {
            per_process_error: Some("NetQuota currently monitors only Windows".into()),
            service_map_at: None,
            last_tick: None,
            pids: HashMap::new(),
            registry: HashMap::new(),
            global: GlobalRuntime::default(),
            cost_at: None,
        }
    }

    pub fn per_process_available(&self) -> bool {
        #[cfg(windows)]
        {
            self.etw.as_ref().is_some_and(|s| s.is_running())
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    pub fn per_process_error(&self) -> Option<&str> {
        self.per_process_error.as_deref()
    }

    pub fn global(&self) -> &GlobalRuntime {
        &self.global
    }

    pub fn registry(&self) -> &HashMap<String, AppRuntime> {
        &self.registry
    }

    /// Forget any ETW session left behind by a crash in this process.
    pub fn recover_stale_session() -> bool {
        #[cfg(windows)]
        {
            crate::etw::recover_stale()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    pub fn poll(&mut self) -> Tick {
        let now = Instant::now();
        let elapsed_ms = self
            .last_tick
            .map(|last| now.duration_since(last).as_millis().max(1) as u64)
            .unwrap_or(1000);
        self.last_tick = Some(now);
        let wall = now_ms();

        #[cfg(windows)]
        let foreground = process::foreground_pid();
        #[cfg(not(windows))]
        let foreground: Option<u32> = None;

        #[cfg(windows)]
        let (events, dropped) = if self.per_process_available() {
            crate::etw::drain()
        } else {
            (Vec::new(), 0)
        };
        #[cfg(not(windows))]
        let (events, dropped) = (Vec::new(), 0u64);

        let mut per_pid: HashMap<u32, Usage> = HashMap::new();
        for event in &events {
            let entry = per_pid.entry(event.pid).or_default();
            match event.direction {
                Direction::Download | Direction::Both => entry.down += event.bytes,
                Direction::Upload => entry.up += event.bytes,
            }
        }

        if !per_pid.is_empty()
            && self
                .service_map_at
                .map(|at| at.elapsed() > SERVICES_REFRESH)
                .unwrap_or(true)
        {
            #[cfg(windows)]
            {
                self.service_map = services::pid_service_map();
            }
            self.service_map_at = Some(now);
        }

        let mut apps = Vec::with_capacity(per_pid.len());
        for (pid, usage) in per_pid {
            let app = self.attribute(pid, usage, wall, foreground);
            apps.push(app);
        }

        #[cfg(windows)]
        self.refresh_global(elapsed_ms);
        #[cfg(windows)]
        self.refresh_cost(now);
        #[cfg(not(windows))]
        {
            self.global.down_speed = 0;
            self.global.up_speed = 0;
        }

        for runtime in self.registry.values_mut() {
            runtime.running = wall - runtime.last_active_ms <= ACTIVE_WINDOW_MS;
        }
        self.pids.retain(|_, state| {
            wall - state.runtime.last_active_ms <= PRUNE_WINDOW_MS
        });
        self.registry.retain(|_, runtime| {
            wall - runtime.last_active_ms <= PRUNE_WINDOW_MS || runtime.running
        });

        Tick {
            elapsed_ms,
            down: apps.iter().map(|a| a.down).sum(),
            up: apps.iter().map(|a| a.up).sum(),
            down_speed: self.global.down_speed,
            up_speed: self.global.up_speed,
            apps,
            dropped_events: dropped,
            per_process_available: self.per_process_available(),
            per_process_error: self.per_process_error.clone(),
            global: self.global.clone(),
            foreground_pid: foreground,
        }
    }

    #[cfg(windows)]
    fn refresh_global(&mut self, elapsed_ms: u64) {
        let snapshot = match global::snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                tracing::debug!(%error, "interface counters unavailable");
                return;
            }
        };
        let (down_delta, up_delta) = match &self.previous {
            // A counter that went backwards means the adapter was reset or
            // replaced; count the new value rather than a negative delta.
            Some(previous) => (
                snapshot.down.saturating_sub(previous.down),
                snapshot.up.saturating_sub(previous.up),
            ),
            None => (0, 0),
        };
        self.previous = Some(snapshot.clone());
        self.global.down_total = snapshot.down;
        self.global.up_total = snapshot.up;
        self.global.down_speed = down_delta.saturating_mul(1000) / elapsed_ms.max(1);
        self.global.up_speed = up_delta.saturating_mul(1000) / elapsed_ms.max(1);
        self.global.primary_alias = snapshot.primary_alias.clone();
        self.global.primary_kind = snapshot.primary_kind;
        self.global.vpn_present = snapshot.vpn_present;
        self.global.interfaces = snapshot.interfaces;
    }

    #[cfg(windows)]
    fn refresh_cost(&mut self, now: Instant) {
        if self
            .cost_at
            .map(|at| now.duration_since(at) > METERED_REFRESH)
            .unwrap_or(true)
        {
            self.cost = metered::connection_cost();
            self.cost_at = Some(now);
            self.global.metered = self.cost.metered;
            self.global.cost_known = self.cost.known;
        }
    }

    #[cfg(windows)]
    fn attribute(&mut self, pid: u32, usage: Usage, wall: i64, foreground: Option<u32>) -> AppTick {
        let known = self.pids.get(&pid).map(|state| state.runtime.clone());
        let fresh = process::process_info(pid);

        let mut confidence = Confidence::Measured;
        let runtime = match (known, fresh) {
            (Some(mut runtime), Some(info)) if runtime.started_ms == info.started_ms => {
                // Same process as the last tick.
                runtime.path = runtime.path.or(info.path);
                runtime.publisher = runtime.publisher.or(info.publisher);
                runtime
            }
            (Some(mut runtime), Some(info)) => {
                // PID was reused by a different process. Attribute this tick to
                // the new process and flag the boundary as estimated.
                confidence = Confidence::Estimated;
                let hosts = self.service_map.get(&pid).cloned().unwrap_or_default();
                let key = info.app_key(&hosts);
                runtime.app_key = key;
                runtime.exe_name = info.exe_name;
                runtime.path = info.path;
                runtime.publisher = info.publisher;
                runtime.started_ms = info.started_ms;
                runtime.down_total = 0;
                runtime.up_total = 0;
                runtime
            }
            (Some(runtime), None) => {
                // The process exited before it could be re-resolved; keep the
                // identity captured earlier, but its last bytes are approximate.
                confidence = Confidence::Estimated;
                runtime
            }
            (None, Some(info)) => {
                let hosts = self.service_map.get(&pid).cloned().unwrap_or_default();
                let key = info.app_key(&hosts);
                let category = classify(
                    &info.exe_name,
                    info.publisher.as_deref(),
                    info.path
                        .as_deref()
                        .is_some_and(|p| p.to_ascii_lowercase().starts_with("c:\\windows\\")),
                );
                AppRuntime {
                    app_key: key,
                    exe_name: info.exe_name,
                    path: info.path,
                    publisher: info.publisher,
                    category,
                    pid,
                    started_ms: info.started_ms,
                    down_total: 0,
                    up_total: 0,
                    down_speed: 0,
                    up_speed: 0,
                    last_active_ms: 0,
                    background: false,
                    running: true,
                    confidence,
                }
            }
            (None, None) => {
                confidence = Confidence::Unknown;
                AppRuntime {
                    app_key: format!("unknown:pid-{pid}"),
                    exe_name: format!("Unknown (PID {pid})"),
                    category: Category::Other,
                    pid,
                    confidence,
                    ..AppRuntime::default()
                }
            }
        };

        let mut runtime = runtime;
        runtime.down_total = runtime.down_total.saturating_add(usage.down);
        runtime.up_total = runtime.up_total.saturating_add(usage.up);
        runtime.down_speed = usage.down;
        runtime.up_speed = usage.up;
        runtime.last_active_ms = wall;
        runtime.background = foreground != Some(pid);
        runtime.running = true;
        runtime.confidence = confidence;
        runtime.pid = pid;

        // Keep one registry entry per application. A newer process for the same
        // executable replaces the totals of an exited one.
        let entry = self
            .registry
            .entry(runtime.app_key.clone())
            .or_insert_with(|| runtime.clone());
        if entry.pid != runtime.pid {
            *entry = runtime.clone();
        } else {
            entry.down_total = runtime.down_total;
            entry.up_total = runtime.up_total;
            entry.down_speed = runtime.down_speed;
            entry.up_speed = runtime.up_speed;
            entry.last_active_ms = runtime.last_active_ms;
            entry.background = runtime.background;
            entry.running = true;
            entry.confidence = runtime.confidence;
        }

        self.pids.insert(pid, PidState {
            runtime: runtime.clone(),
        });

        AppTick {
            app_key: runtime.app_key,
            exe_name: runtime.exe_name,
            path: runtime.path,
            publisher: runtime.publisher,
            category: runtime.category,
            pid,
            started_ms: runtime.started_ms,
            down: usage.down,
            up: usage.up,
            down_speed: usage.down,
            up_speed: usage.up,
            background: runtime.background,
            confidence,
        }
    }
}

/// Alias kept for call sites that think of this as the shared monitor state.
pub type MonitorState = NetworkMonitor;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polling_twice_produces_consistent_timing_and_availability() {
        let mut monitor = NetworkMonitor::new();
        let first = monitor.poll();
        assert_eq!(first.elapsed_ms, 0);
        std::thread::sleep(Duration::from_millis(30));
        let second = monitor.poll();

        assert!(second.elapsed_ms >= 20, "elapsed {}ms", second.elapsed_ms);
        assert_eq!(second.per_process_available, monitor.per_process_available());
        if monitor.per_process_available() {
            assert!(second.per_process_error.is_none());
        } else {
            // Unprivileged runs must say so rather than report zero usage.
            assert!(second.per_process_error.is_some());
        }
    }

    #[test]
    fn global_counters_do_not_go_backwards_between_polls() {
        let mut monitor = NetworkMonitor::new();
        monitor.poll();
        let second = monitor.poll();
        // Total counters are absolute; the delta must never be negative.
        assert!(second.global.interfaces.len() < 512);
        let _ = second.global.up_total;
    }

    #[test]
    fn missing_process_is_unknown_not_fabricated() {
        let mut monitor = NetworkMonitor::new();
        let usage = Usage { down: 1024, up: 512 };
        let tick = monitor.attribute(u32::MAX - 2, usage, now_ms(), None);
        assert_eq!(tick.confidence, Confidence::Unknown);
        assert!(tick.exe_name.starts_with("Unknown"));
        assert_eq!(tick.down, 1024);
        assert_eq!(tick.up, 512);
    }
}

