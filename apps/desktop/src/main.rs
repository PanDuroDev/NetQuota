//! Tauri shell for NetQuota. The UI is unelevated and talks to the service over
//! the same named pipe as the CLI and the Chrome bridge. When the service is not
//! running, only interface-level totals are available and the UI says so.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::sync::Mutex;

use netquota_core::period::PeriodKind;
use netquota_core::policy::{AppPolicy, Enforcement};
use netquota_core::profile::ProfileKind;
use netquota_core::quota::Limit;
use netquota_ipc::Client;
use netquota_protocol::wire::{AppUsage, EventRecord, GlobalUsage, SiteUsage, Snapshot};
use netquota_protocol::{Request, Response, PROTOCOL_VERSION};
use serde::Serialize;
use tauri::State;

#[derive(Default)]
struct IconCache(Mutex<HashMap<String, Option<Vec<u8>>>>);

fn call(request: Request) -> Result<Response, String> {
    let mut client = Client::connect(400).map_err(|err| format!("service unavailable: {err}"))?;
    let response = client
        .call(&request)
        .map_err(|err| format!("service call failed: {err}"))?;
    match response {
        Response::Error { code, message } => Err(format!("{code:?}: {message}")),
        other => Ok(other),
    }
}

#[derive(Serialize)]
struct Health {
    connected: bool,
    service_version: Option<String>,
    monitoring: Option<String>,
    enforcement_ready: bool,
    protocol_version: u16,
    pipe: String,
}

#[tauri::command]
fn health() -> Health {
    match call(Request::Hello {
        version: PROTOCOL_VERSION,
        client: netquota_protocol::ClientKind::Desktop,
        extension_id: None,
    }) {
        Ok(Response::Hello {
            version,
            service_version,
            enforcement_ready,
            monitoring,
        }) => Health {
            connected: true,
            service_version: Some(service_version),
            monitoring: Some(format!("{monitoring:?}").to_ascii_lowercase()),
            enforcement_ready,
            protocol_version: version,
            pipe: netquota_ipc::PIPE_NAME.to_string(),
        },
        _ => Health {
            connected: false,
            service_version: None,
            monitoring: None,
            enforcement_ready: false,
            protocol_version: PROTOCOL_VERSION,
            pipe: netquota_ipc::PIPE_NAME.to_string(),
        },
    }
}

#[tauri::command]
fn snapshot() -> Result<Snapshot, String> {
    match call(Request::Snapshot)? {
        Response::Snapshot { snapshot } => Ok(snapshot),
        _ => Err("unexpected response".into()),
    }
}

#[derive(Serialize)]
struct FallbackUsage {
    down: u64,
    up: u64,
    primary_alias: Option<String>,
    primary_kind: Option<String>,
}

/// Interface totals work without the service, so the dashboard is never empty
/// just because the service is not installed. Per-app rows stay absent.
#[tauri::command]
fn fallback_usage() -> Result<FallbackUsage, String> {
    let snap = netquota_monitor::global::snapshot().map_err(|err| err.to_string())?;
    Ok(FallbackUsage {
        down: snap.down,
        up: snap.up,
        primary_alias: snap.primary_alias,
        primary_kind: snap.primary_kind.map(|k| k.as_str().to_string()),
    })
}

#[tauri::command]
fn apps() -> Result<Vec<AppUsage>, String> {
    match call(Request::ListApps)? {
        Response::Apps { apps } => Ok(apps),
        _ => Err("unexpected response".into()),
    }
}

#[tauri::command]
fn sites() -> Result<Vec<SiteUsage>, String> {
    match call(Request::ListSites)? {
        Response::Sites { sites } => Ok(sites),
        _ => Err("unexpected response".into()),
    }
}

#[tauri::command]
fn events(limit: u32) -> Result<Vec<EventRecord>, String> {
    match call(Request::ListEvents { limit })? {
        Response::Events { events } => Ok(events),
        _ => Err("unexpected response".into()),
    }
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
fn save_policy(
    app_key: String,
    limit_bytes: Option<u64>,
    period: PeriodKind,
    enforcement: Enforcement,
    block_background: bool,
    foreground_only: bool,
    block_on_metered: bool,
    wifi_only: bool,
    paused: bool,
) -> Result<(), String> {
    let mut policy = AppPolicy::new(app_key);
    policy.limit = Limit {
        total: limit_bytes,
        download: None,
        upload: None,
    };
    policy.period = period;
    policy.enforcement = enforcement;
    policy.background.block_in_background = block_background;
    policy.background.foreground_only = foreground_only;
    policy.background.block_on_metered = block_on_metered;
    policy.background.wifi_only = wifi_only;
    policy.paused = paused;
    policy.validate().map_err(|err| err.to_string())?;
    match call(Request::SetPolicy { policy })? {
        Response::PolicyOk => Ok(()),
        _ => Err("unexpected response".into()),
    }
}

#[tauri::command]
fn set_paused(app_key: String, paused: bool) -> Result<(), String> {
    match call(Request::SetPaused { app_key, paused })? {
        Response::PolicyOk => Ok(()),
        _ => Err("unexpected response".into()),
    }
}

#[tauri::command]
fn grant_credit(app_key: String, extra_bytes: u64, duration_secs: Option<u64>) -> Result<(), String> {
    match call(Request::GrantCredit {
        app_key,
        extra_bytes,
        duration_secs,
    })? {
        Response::PolicyOk => Ok(()),
        _ => Err("unexpected response".into()),
    }
}

#[tauri::command]
fn set_blocked(app_key: String, blocked: bool) -> Result<(), String> {
    let request = if blocked {
        Request::BlockNow { app_key }
    } else {
        Request::Unblock { app_key }
    };
    match call(request)? {
        Response::PolicyOk => Ok(()),
        _ => Err("unexpected response".into()),
    }
}

#[tauri::command]
fn apply_profile(profile: ProfileKind) -> Result<(), String> {
    match call(Request::ApplyProfile { profile })? {
        Response::PolicyOk => Ok(()),
        _ => Err("unexpected response".into()),
    }
}

#[tauri::command]
fn set_site_policy(site: String, limit_bytes: Option<u64>, blocked: bool) -> Result<(), String> {
    match call(Request::SetSitePolicy {
        site,
        limit_bytes,
        blocked,
    })? {
        Response::PolicyOk => Ok(()),
        _ => Err("unexpected response".into()),
    }
}

#[tauri::command]
fn restore_all() -> Result<u32, String> {
    match call(Request::EmergencyRestore)? {
        Response::Restored { removed_rules } => Ok(removed_rules),
        _ => Err("unexpected response".into()),
    }
}

/// Installs the service through a UAC prompt using the executable that ships
/// next to this app. Re-running it upgrades the registration in place.
#[tauri::command]
fn install_service() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|err| err.to_string())?;
    let service = exe
        .parent()
        .ok_or("cannot locate the app directory")?
        .join("netquota-service.exe");
    if !service.is_file() {
        return Err(format!("netquota-service.exe not found next to {}", exe.display()));
    }
    let script = format!(
        "Start-Process -FilePath '{}' -ArgumentList '--install' -Verb RunAs -Wait",
        service.display().to_string().replace('\'', "''")
    );
    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .status()
        .map_err(|err| err.to_string())?;
    if !status.success() {
        return Err("installation was cancelled or failed".into());
    }
    Ok("service installed".into())
}

/// Registers the native messaging host for this Windows user and authorizes the
/// extension id in the service. The service part needs admin, so it runs through
/// the same UAC helper as install.
#[tauri::command]
fn register_extension(extension_id: String) -> Result<String, String> {
    let id = extension_id.trim().to_ascii_lowercase();
    if id.len() != 32 || !id.bytes().all(|byte| (b'a'..=b'p').contains(&byte)) {
        return Err("extension id must be 32 characters in a-p".into());
    }
    let dir = std::env::current_exe()
        .map_err(|err| err.to_string())?
        .parent()
        .ok_or("cannot locate the app directory")?
        .to_path_buf();
    let host = dir.join("netquota-native-host.exe");
    let service = dir.join("netquota-service.exe");
    if !host.is_file() || !service.is_file() {
        return Err("native host and service executables must sit next to the app".into());
    }
    let local = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .ok_or("LOCALAPPDATA is not set")?
        .join("NetQuota");
    std::fs::create_dir_all(&local).map_err(|err| err.to_string())?;
    let manifest = local.join("com.netquota.host.json");
    let body = serde_json::json!({
        "name": "com.netquota.host",
        "description": "NetQuota native messaging bridge",
        "path": host.to_string_lossy(),
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{id}/")],
    });
    let text = serde_json::to_string_pretty(&body).map_err(|err| err.to_string())?;
    std::fs::write(&manifest, text).map_err(|err| err.to_string())?;

    let key = r"HKCU\Software\Google\Chrome\NativeMessagingHosts\com.netquota.host";
    let status = std::process::Command::new("reg")
        .args(["add", key, "/ve", "/t", "REG_SZ", "/d"])
        .arg(&manifest)
        .arg("/f")
        .status()
        .map_err(|err| err.to_string())?;
    if !status.success() {
        return Err("could not write the native messaging registry key".into());
    }

    let script = format!(
        "Start-Process -FilePath '{}' -ArgumentList '--allow-extension','{id}' -Verb RunAs -Wait",
        service.display().to_string().replace('\'', "''")
    );
    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .status()
        .map_err(|err| err.to_string())?;
    if !status.success() {
        return Err("authorizing the extension was cancelled or failed".into());
    }
    Ok("extension registered".into())
}

/// Returns PNG bytes for a desktop app icon, extracted from the executable by the
/// same code path the service uses. Cached per path for the session.
#[tauri::command]
fn app_icon(path: String, cache: State<'_, IconCache>) -> Option<Vec<u8>> {
    if !path.starts_with(|c: char| c.is_ascii_alphabetic())
        || !path.to_ascii_lowercase().ends_with(".exe")
        || path.contains('"')
        || path.contains('\n')
    {
        return None;
    }
    if let Ok(guard) = cache.0.lock() {
        if let Some(hit) = guard.get(&path) {
            return hit.clone();
        }
    }
    let icon = netquota_monitor::icons::icon_png(&path);
    if let Ok(mut guard) = cache.0.lock() {
        guard.insert(path, icon.clone());
    }
    icon
}

#[tauri::command]
fn global_usage() -> Result<GlobalUsage, String> {
    let snap = netquota_monitor::global::snapshot().map_err(|err| err.to_string())?;
    Ok(GlobalUsage {
        down: snap.down,
        up: snap.up,
        today_down: 0,
        today_up: 0,
        down_bps: 0.0,
        up_bps: 0.0,
    })
}

fn main() {
    tauri::Builder::default()
        .manage(IconCache::default())
        .invoke_handler(tauri::generate_handler![
            health,
            snapshot,
            fallback_usage,
            apps,
            sites,
            events,
            save_policy,
            set_paused,
            grant_credit,
            set_blocked,
            apply_profile,
            set_site_policy,
            restore_all,
            install_service,
            register_extension,
            app_icon,
            global_usage
        ])
        .run(tauri::generate_context!())
        .expect("failed to start NetQuota desktop");
}
