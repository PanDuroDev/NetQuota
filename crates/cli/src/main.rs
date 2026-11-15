//! Small operator CLI. Every network-changing command goes through the service
//! when it is running; `recover` also works without it, because that is the
//! command a user needs when the service is the thing that failed.

use std::path::Path;

use anyhow::{bail, Result};
use netquota_core::units::format_bytes;
use netquota_ipc::Client;
use netquota_protocol::wire::Snapshot;
use netquota_protocol::{ClientKind, Request, Response, PROTOCOL_VERSION};

const USAGE: &str = "\
netquota-cli <command> [--json]

  status            service status, today's usage, active restrictions
  apps              per-application usage
  sites             website usage reported by the extension
  events [n]        recent quota and enforcement events
  ifstats [--watch] read interface counters directly (works without the service)
  block <path>      block an executable now
  unblock <path>    remove its NetQuota block
  recover           remove every NetQuota firewall rule (works offline)
  doctor            environment and capability check
";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json = args.iter().any(|a| a == "--json");
    let command = args.first().map(String::as_str).unwrap_or("status");
    match command {
        "status" => status(json),
        "apps" => apps(json),
        "sites" => sites(json),
        "events" => {
            let limit = args.get(1).and_then(|n| n.parse().ok()).unwrap_or(30);
            events(limit, json)
        }
        "ifstats" => ifstats(args.iter().any(|a| a == "--watch"), json),
        "block" => enforce(args.get(1), true),
        "unblock" => enforce(args.get(1), false),
        "recover" => recover(),
        "doctor" => doctor(),
        "help" | "--help" | "-h" => {
            println!("{USAGE}");
            Ok(())
        }
        other => bail!("unknown command {other:?}\n\n{USAGE}"),
    }
}

fn connect() -> Result<Client> {
    let mut client = Client::connect(2_000)?;
    let hello = Request::Hello {
        version: PROTOCOL_VERSION,
        client: ClientKind::Cli,
        extension_id: None,
    };
    match client.call(&hello)? {
        Response::Hello { service_version, monitoring, enforcement_ready, .. } => {
            tracing_note(format!(
                "service {service_version}, monitoring: {monitoring:?}, enforcement: {}",
                if enforcement_ready { "live" } else { "dry-run" }
            ));
            Ok(client)
        }
        Response::Error { message, .. } => bail!("service rejected the session: {message}"),
        _ => bail!("unexpected handshake response"),
    }
}

fn tracing_note(message: String) {
    eprintln!("{message}");
}

fn status(json: bool) -> Result<()> {
    let mut client = connect()?;
    let snapshot = match client.call(&Request::Snapshot)? {
        Response::Snapshot { snapshot } => snapshot,
        other => bail!("unexpected response: {other:?}"),
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
        return Ok(());
    }
    print_snapshot(&snapshot);
    Ok(())
}

fn print_snapshot(snapshot: &Snapshot) {
    let g = &snapshot.global;
    println!("NetQuota {}", snapshot.service_version);
    println!(
        "  monitoring     : {:?}{}",
        snapshot.monitoring,
        if snapshot.enforcement_ready { "" } else { " (firewall dry-run)" }
    );
    println!("  interface      : {}", snapshot.interface.as_deref().unwrap_or("none"));
    println!("  metered        : {}", if snapshot.metered { "yes" } else { "no" });
    println!("  profile        : {}", snapshot.profile);
    println!("  today          : down {} / up {}", format_bytes(g.today_down), format_bytes(g.today_up));
    println!("  speed          : {} /s down, {} /s up", format_bytes(g.down_bps as u64), format_bytes(g.up_bps as u64));
    println!("  since start    : down {} / up {}", format_bytes(g.down), format_bytes(g.up));
    println!("  applications   : {}", snapshot.apps.len());
    let restricted = snapshot.apps.iter().filter(|a| a.blocked).count();
    println!("  restricted     : {restricted}");
    if snapshot.unattributed_bytes > 0 {
        println!("  dropped events : {}", snapshot.unattributed_bytes);
    }
    for app in snapshot.apps.iter().filter(|a| a.blocked || a.exhausted).take(10) {
        println!(
            "  ! {:<28} {}{}",
            app.identity.exe_name,
            if app.blocked { "blocked " } else { "" },
            if app.exhausted { "quota exhausted" } else { "" }
        );
    }
}

fn apps(json: bool) -> Result<()> {
    let mut client = connect()?;
    let apps = match client.call(&Request::ListApps)? {
        Response::Apps { apps } => apps,
        other => bail!("unexpected response: {other:?}"),
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&apps)?);
        return Ok(());
    }
    println!(
        "{:<30} {:>10} {:>10} {:>12} {:>12} {:>10}  {}",
        "application", "down", "up", "period", "limit", "speed", "state"
    );
    for app in apps {
        println!(
            "{:<30} {:>10} {:>10} {:>12} {:>12} {:>10}  {}{}{}",
            truncate(&app.identity.exe_name, 30),
            format_bytes(app.down),
            format_bytes(app.up),
            format_bytes(app.period_used.total()),
            app.limit.total.map(format_bytes).unwrap_or_else(|| "unlimited".into()),
            format!("{}/s", format_bytes(app.speed_down_bps as u64)),
            if app.blocked { "blocked" } else if app.paused { "paused" } else if app.background { "background" } else { "ok" },
            if app.exhausted { " (quota)" } else { "" },
            if app.protection_note.is_some() { " [protected]" } else { "" }
        );
    }
    Ok(())
}

fn sites(json: bool) -> Result<()> {
    let mut client = connect()?;
    let sites = match client.call(&Request::ListSites)? {
        Response::Sites { sites } => sites,
        other => bail!("unexpected response: {other:?}"),
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&sites)?);
        return Ok(());
    }
    for site in sites {
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>8}  {}{}",
            site.host,
            format_bytes(site.down),
            format_bytes(site.up),
            site.limit.total.map(format_bytes).unwrap_or_else(|| "unlimited".into()),
            format!("{}s", site.active_secs),
            if site.estimated { "estimated" } else { "measured" },
            if site.blocked { " blocked" } else { "" }
        );
    }
    Ok(())
}

fn events(limit: u32, json: bool) -> Result<()> {
    let mut client = connect()?;
    let events = match client.call(&Request::ListEvents { limit })? {
        Response::Events { events } => events,
        other => bail!("unexpected response: {other:?}"),
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&events)?);
        return Ok(());
    }
    for event in events {
        let when = chrono::DateTime::from_timestamp_millis(event.ts_ms)
            .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| event.ts_ms.to_string());
        println!("{when}  {:<18} {:<28} {}", format!("{:?}", event.kind), truncate(&event.subject, 28), event.message);
    }
    Ok(())
}

fn ifstats(watch: bool, json: bool) -> Result<()> {
    let mut previous = netquota_monitor::global::snapshot()?;
    loop {
        if watch {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
        let current = netquota_monitor::global::snapshot()?;
        let down = current.down.saturating_sub(previous.down);
        let up = current.up.saturating_sub(previous.up);
        if json {
            let value = serde_json::json!({
                "down": current.down,
                "up": current.up,
                "delta_down": down,
                "delta_up": up,
                "interface": current.primary_alias,
                "kind": current.primary_kind.map(|k| k.as_str()),
                "vpn": current.vpn_present,
            });
            println!("{value}");
        } else {
            println!(
                "total down {} up {} | now {}/s down {}/s up | {}",
                format_bytes(current.down),
                format_bytes(current.up),
                format_bytes(down),
                format_bytes(up),
                current.primary_alias.as_deref().unwrap_or("no interface")
            );
            for interface in &current.interfaces {
                println!(
                    "  {:<32} {:<8} in {} out {}",
                    truncate(&interface.alias, 32),
                    interface.kind.as_str(),
                    format_bytes(interface.in_octets),
                    format_bytes(interface.out_octets)
                );
            }
        }
        if !watch {
            return Ok(());
        }
        previous = current;
    }
}

fn enforce(path: Option<&String>, block: bool) -> Result<()> {
    let Some(path) = path else {
        bail!("usage: netquota-cli {} <full-path-to-exe>", if block { "block" } else { "unblock" });
    };
    let canonical = Path::new(path);
    if !canonical.is_absolute() {
        bail!("path must be absolute: {path}");
    }
    let mut client = connect()?;
    let request = if block {
        Request::BlockNow { app_key: key_for(canonical) }
    } else {
        Request::Unblock { app_key: key_for(canonical) }
    };
    match client.call(&request)? {
        Response::PolicyOk => {
            println!("{} {}", if block { "blocked" } else { "unblocked" }, canonical.display());
            Ok(())
        }
        Response::Error { message, .. } => bail!("{message}"),
        other => bail!("unexpected response: {other:?}"),
    }
}

fn key_for(path: &Path) -> String {
    path.to_string_lossy().to_ascii_lowercase()
}

fn recover() -> Result<()> {
    // Try the service first: it owns the manifest and can report precisely.
    if let Ok(mut client) = Client::connect(1_000) {
        let _ = client.call(&Request::Hello {
            version: PROTOCOL_VERSION,
            client: ClientKind::Cli,
            extension_id: None,
        });
        if let Ok(Response::Restored { removed_rules }) = client.call(&Request::EmergencyRestore) {
            println!("restored network access ({removed_rules} rule(s) removed)");
            return Ok(());
        }
    }
    // Offline path: the service is stopped or broken.
    let manifest = netquota_service_data_dir().join("firewall-manifest.json");
    let mut firewall = netquota_enforcement::Firewall::connect(manifest, false)?;
    let removed = firewall.restore_all()?;
    println!("restored network access offline ({removed} rule(s) removed)");
    Ok(())
}

fn netquota_service_data_dir() -> std::path::PathBuf {
    std::env::var("NETQUOTA_DATA_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from(r"C:\ProgramData\NetQuota"))
}

fn doctor() -> Result<()> {
    println!("NetQuota doctor\n");
    println!("  data dir            : {}", netquota_service_data_dir().display());
    println!("  protocol version    : {PROTOCOL_VERSION}");

    match Client::connect(500) {
        Ok(mut client) => match client.call(&Request::Hello {
            version: PROTOCOL_VERSION,
            client: ClientKind::Cli,
            extension_id: None,
        }) {
            Ok(Response::Hello { service_version, monitoring, enforcement_ready, .. }) => {
                println!("  service             : reachable ({service_version})");
                println!("  monitoring          : {monitoring:?}");
                println!("  enforcement         : {}", if enforcement_ready { "live" } else { "dry-run" });
            }
            Ok(other) => println!("  service             : unexpected {other:?}"),
            Err(e) => println!("  service             : handshake failed: {e}"),
        },
        Err(_) => println!("  service             : not reachable (pipe {})", netquota_ipc::PIPE_NAME),
    }

    match netquota_monitor::global::snapshot() {
        Ok(snapshot) => println!(
            "  interfaces          : {} (primary {})",
            snapshot.interfaces.len(),
            snapshot.primary_alias.as_deref().unwrap_or("none")
        ),
        Err(e) => println!("  interfaces          : unreadable: {e}"),
    }

    let mut monitor = netquota_monitor::NetworkMonitor::new();
    let tick = monitor.poll();
    println!(
        "  per-process ETW     : {}{}",
        if tick.per_process_available { "available" } else { "unavailable" },
        tick.per_process_error.as_deref().map(|e| format!(" ({e})")).unwrap_or_default()
    );

    let manifest = netquota_service_data_dir().join("firewall-manifest.json");
    match netquota_enforcement::Firewall::connect(manifest, true) {
        Ok(firewall) => println!("  firewall rules owned: {}", firewall.list_owned().len()),
        Err(e) => println!("  firewall            : unreadable: {e}"),
    }

    let extension_file = netquota_service_data_dir().join("extension-id.txt");
    println!(
        "  extension allow-list: {}",
        if extension_file.exists() { extension_file.display().to_string() } else { "missing".into() }
    );
    Ok(())
}

fn truncate(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        value.to_string()
    } else {
        value.chars().take(width.saturating_sub(1)).collect::<String>() + "…"
    }
}

