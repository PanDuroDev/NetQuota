# NetQuota architecture

NetQuota is a local-first Windows data manager. Three processes cooperate, and
none of them requires an account, a backend, or an internet connection after
installation.

```
┌──────────────────────┐   named pipe (JSON frames)   ┌──────────────────────────┐
│ Desktop app (Tauri)  │ ───────────────────────────► │  NetQuota service        │
│ unelevated, per-user │ ◄─────────────────────────── │  LocalSystem, per-machine│
└──────────────────────┘                              └───────────┬──────────────┘
                                                                  │ ETW, IfTable2, SCM,
                                                                  │ Windows Firewall COM
┌──────────────────────┐   native messaging (stdio)               │
│ Chrome extension     │ ◄──────────────► netquota-native-host ───┘ (pipe client)
│ MV3 service worker   │
└──────────────────────┘
```

## Why a service

Per-process byte accounting needs ETW (`Microsoft-Windows-Kernel-Network`), and
changing firewall rules needs elevation. Both require administrator rights, so
they live in one LocalSystem service. The UI and the extension never run
elevated: they send validated requests over a named pipe and can do nothing the
service does not expose.

## Components

| Path | Role |
| --- | --- |
| `apps/desktop` | Tauri 2 shell, vanilla TypeScript UI, pipe client, icon cache |
| `apps/chrome-extension` | MV3 extension: site ledger, DNR blocking, video adapters |
| `crates/core` | Pure domain types: identity, periods, quotas, policies, profiles, rules, units |
| `crates/protocol` | Wire types + length-prefixed framing shared by pipe and native host |
| `crates/storage` | SQLite schema, migrations, aggregated usage queries, backups |
| `crates/monitor` | Global interface counters, ETW per-process accounting, process identity, SCM map, icons, metered state |
| `crates/enforcement` | Firewall rule ownership and apply/remove; speed-limit capability reporting |
| `crates/quota-engine` | Pure quota decisions with hysteresis and transitions |
| `crates/ipc` | Named pipe server/client with an explicit SDDL |
| `crates/windows-service` | Service host, runtime loop, persistence, notifications, recovery |
| `crates/native-host` | Chrome native messaging bridge with origin + schema validation |
| `crates/cli` | Diagnostics, `doctor`, offline recovery, machine-readable output |

## Data flow

1. The service polls the monitor once per second. Interface counters give the
   global totals; ETW events give per-process deltas.
2. Deltas are accumulated in memory and written to SQLite every five seconds as
   hourly buckets (`usage_hourly`). Writing every packet would cost more than the
   monitoring itself.
3. Every two seconds the quota engine evaluates each app's current period window
   against its policy and credit. Transitions (not steady states) produce events;
   only transitions can trigger a block or a notification.
4. Blocks are applied through the Windows Firewall COM API as outbound rules
   named `NetQuota\Block\<hash>` and tracked in `firewall-manifest.json`. Only
   names in that manifest are ever removed.
5. The desktop app polls a snapshot over the pipe (1 s) and renders it. The
   extension pushes estimated site totals; the service stores them as
   `estimated`, never as measured.
6. On stop, crash-recovery start, or `netquota-cli recover`, the service removes
   every NetQuota rule so a failure cannot leave the machine offline.

## Trust boundaries

- Pipe: DACL grants `SY` and `BA` full control and interactive users read/write,
  with remote pipe clients rejected. Requests are schema-validated; unknown or
  malformed frames return `BadRequest`.
- Native host: rejects any origin that is not `chrome-extension://<id>/`, checks
  the id against `extension-id.txt`, and enforces a 1 MB message ceiling in both
  directions.
- Content scripts: they can report usage and read settings, but have no path to
  privileged operations. Blocking is a declarativeNetRequest rule created by the
  worker.
- The extension never intercepts HTTPS, never installs a root certificate, and
  never touches DRM.

## Failure modes

| Failure | Behaviour |
| --- | --- |
| Service not running | Desktop shows interface-level usage directly; extension keeps its own rules and shows a disconnected badge |
| ETW unavailable (not admin) | Monitoring reports `degraded`, per-process rows are absent, never fabricated |
| Process exits during accounting | Totals are kept; last activity is shown; entry is pruned after inactivity |
| Database write fails | The pending batch is retained and retried |
| Firewall rule fails | Rule is not recorded; a warning event is stored, no false "blocked" state |
| Service killed while blocking | `recover` and the next service start remove owned rules |

## Deliberate omissions

- No kernel driver. Blocking uses supported Windows facilities; shaping is
  limited (see `ENFORCEMENT.md`).
- No cloud sync, telemetry, or account system.
- No packet inspection. NetQuota counts and restricts; it does not read payloads.
- macOS/Linux are not supported. The provider traits exist so a port is possible,
  but nothing is claimed until it is implemented and tested.
