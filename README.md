# NetQuota

Local-first internet usage control for Windows: per-application monitoring,
quotas that actually block, and a Chrome extension that brings the same idea to
websites and video.

No account. No cloud. No telemetry. One machine, one SQLite file.

## What works today

| Capability | State |
| --- | --- |
| Global usage (down/up, speed, interface, VPN detection) | Working, exact adapter counters |
| Per-process usage | Working through ETW; requires the installed service (LocalSystem) |
| Application identity, publisher, icons | Working |
| Daily/weekly/monthly/custom quotas, per-direction limits, credits | Working |
| Enforcement through owned firewall rules, with recovery | Working |
| Events and in-app notifications | Working |
| Chrome extension: site ledger, DNR blocking, blocked page, temp access | Working |
| Native messaging bridge with origin allow-list | Working |
| Video adapters: YouTube, TikTok, generic | Working with honest support levels |
| Desktop UI (Tauri 2, en/ar, dark/light) | Working |
| Upload/download speed limits | **Not enforced** — fields exist, provider reports unsupported |
| Download shaping | **Not implemented** — needs a WFP driver |
| Automation rule editor | Engine exists, editor is v0.2 |
| MSI installer | Scripts only (`scripts/install-service.ps1`) |

Anything in the last four rows is a known gap, not a hidden failure. See
[ROADMAP.md](ROADMAP.md) and [docs/technical-research/FEASIBILITY.md](docs/technical-research/FEASIBILITY.md).

## How it fits together

```
Desktop (Tauri, unelevated) ──┐
Chrome extension ── native host ── named pipe ── NetQuota service (LocalSystem)
                                                   ├── ETW per-process accounting
                                                   ├── interface counters
                                                   └── Windows Firewall rules
```

The service is the only elevated component. It monitors, decides, and enforces;
the UI and browser can only ask it to do things through a validated, DACL-restricted
pipe.

## Quick start (developer)

Requirements: Windows 10/11, Rust 1.75+, Node 20+, an elevated PowerShell only
for the service step.

```powershell
git clone https://github.com/PanDuroDev/NetQuota
cd NetQuota
cargo test --workspace
cargo build --release

# Run the service in the foreground, no install, dry-run enforcement:
$env:NETQUOTA_DATA_DIR = "$env:TEMP\netquota"
$env:NETQUOTA_DRY_RUN = "1"
.\target\release\netquota-service.exe --console

# In another shell:
.\target\release\netquota-cli.exe status
.\target\release\netquota-cli.exe doctor
```

Global usage works in that console session. Per-process rows need the session to
be able to open an ETW kernel session, which needs an elevated service — `doctor`
will tell you if it cannot.

Desktop app:

```powershell
cd apps\desktop\ui; npm install; npx tsc -p tsconfig.json; cd ..\..
cargo run -p netquota-desktop
```

Chrome extension:

```powershell
.\scripts\build-extension.ps1
# chrome://extensions -> Developer mode -> Load unpacked -> apps\chrome-extension
# then, elevated, install the service and register the host:
.\scripts\install-service.ps1 -ExtensionId <the 32-character id>
```

## Testing

```powershell
cargo test --workspace
cd apps\chrome-extension; npm run typecheck
```

The suite covers quota boundaries, DST-safe period windows, protocol framing,
storage aggregation, PID reuse, firewall rule ownership, pipe round trips, native
host validation, and quota-engine transitions. Tests never download files or
leave firewall rules behind. Elevated scenario tests are documented in
[docs/testing/TESTING.md](docs/testing/TESTING.md).

## Measurement honesty

- Global totals come from NDIS interface counters, excluding loopback and
  duplicate filter layers. On the development machine that correction changed a
  wrong 51.81 GB into the correct 12.96 GB.
- Per-process numbers come from ETW data events, excluding retransmissions to
  avoid double counting. If the PID cannot be resolved, the app is shown as
  `Unknown` with `confidence: unknown`; nothing is invented.
- Website bytes are **estimated**. Cross-origin media does not expose transfer
  sizes, so the extension combines measured resources with a documented bitrate
  model. Both UIs label it as estimated.
- Video quality is `enforced` only when read back from the player. Otherwise the
  adapter reports `best_effort` or `unsupported`.

## Privacy

Stored locally: app identities, policies, hourly usage, site totals, events,
settings. Never stored: page contents, cookies, passwords, form data, message
content. Nothing is uploaded. The extension requests no account and contacts no
external service; icons come from the executable or the current tab's favicon.

## Repository layout

See [docs/architecture/OVERVIEW.md](docs/architecture/OVERVIEW.md). Crates are
split by responsibility rather than by line count; `core` and `quota-engine`
contain no Windows code and are fully unit-testable.

## License

MIT. See [LICENSE](LICENSE).
