# Feasibility research

Each question was answered with a probe on Windows, not from documentation alone.
Where a capability could not be verified, the feature is marked accordingly.

## 1. Per-process byte accounting

| Candidate | Verdict |
| --- | --- |
| ETW `Microsoft-Windows-Kernel-Network` | **Selected.** Real-time TCP/UDP v4/v6 send/recv with PID. Needs admin / Performance Log Users. |
| `GetExtendedTcpTable` | Ownership only, no bytes. Rejected for quotas. |
| `GetPerTcpConnectionEStats` | TCP only, no UDP/QUIC, sampling, per-connection cost. Rejected. |
| WFP callout + kernel driver | Exact and shapeable, but requires a signed driver. Deferred. |
| Interface counters only | Exact totals, no attribution. Kept as the global provider and as the unprivileged fallback. |

Probe result on this machine: unelevated `StartTraceW` fails with **error 5**;
the service (LocalSystem) starts the session. The monitor therefore reports
`degraded` with no per-process rows when run unelevated, and never invents rows.

## 2. Double counting

Real measurement caught a 4× error: `GetIfTable2` returned 51.81 GB while the
physical adapter reported 12.96 GB. Cause: NDIS filter layers (WFP native, QoS
packet scheduler) repeat the same counters. Filtering layered aliases/descriptions
brought totals in line. Retransmit ETW events (14/30) are excluded for the same
reason.

## 3. Application blocking without a driver

Windows Firewall COM works, is present on every supported Windows version, and
supports per-executable rules. Verified: rule creation, group assignment,
idempotent naming, removal by manifest, and that unknown names are skipped.
Chosen over WFP callouts because it needs no kernel component and is trivially
reversible by the user.

## 4. Bidirectional speed limiting

Not feasible with supported user-mode APIs.

- QoS `ThrottleRateActionBitsPerSecond` is outbound-only. A "download limit"
  built on it would silently do nothing.
- No user-mode API throttles received bytes per process.
- True shaping requires a WFP callout driver.

Decision: speed fields are stored, displayed, and documented as unenforced in
v0.1. The UI does not claim otherwise.

## 5. Chrome native messaging

Verified end-to-end:

- Manifest registration under `HKCU` works for an unpacked extension.
- Chrome passes the caller origin as `argv[1]`; strict validation rejects
  anything that is not `chrome-extension://<32×a-p>/`.
- 4-byte length framing matches the pipe framing, so the host is a thin bridge.
- Disconnect is observable and reconnect works; the extension degrades to local
  enforcement when the host is absent.

## 6. Chrome website blocking

`declarativeNetRequest` dynamic rules work per `requestDomains`, support
`main_frame` redirect and per-type block, and survive service-worker restarts.
`webRequest` blocking is unavailable in MV3 for this use, and host-based blocking
is the only attribution that does not risk collateral damage to shared CDNs.

## 7. Site byte measurement

`PerformanceResourceTiming` exposes `transferSize` only for same-origin or
Timing-Allow-Origin resources. Cross-origin media reports 0, so exact per-site
bytes are **not** obtainable in an extension. The extension reports an estimate
from measured resources plus a documented bitrate model, flagged `estimated`.

## 8. YouTube quality control

The IFrame API `setPlaybackQuality` no longer sets quality. The on-page player
still exposes `setPlaybackQualityRange` and `getPlaybackQuality`, so the adapter
can request a level and then **verify** it. When the player ignores the request,
the adapter reports `best_effort`. This is the highest support level that can be
honestly claimed without intercepting media manifests.

## 9. TikTok quality control

No stable page-level quality API. Quality forcing is `unsupported`. Autoplay,
preload, off-screen, and hidden-tab reductions are deterministic and reported
`enforced`.

## 10. Metered connection detection

`NetworkList` COM is not exposed by `windows` 0.58. WinRT
`NetworkInformation::GetInternetConnectionProfile` →
`GetConnectionCost().NetworkCostType()` works; `Fixed`/`Variable` are treated as
metered, `Unrestricted` is not, and an unknown profile yields `known: false`
rather than a guess.

## Acceptance criteria for v0.1

| Criterion | Status |
| --- | --- |
| Real global usage displayed | Met — interface counters, verified against adapter bytes |
| Per-process usage | Implemented via ETW; requires the installed service (admin), reported `degraded` otherwise |
| Application identity + icons | Met — path/creation time/publisher, PNG icons from the executable |
| Quota configuration | Met — daily/weekly/monthly/custom, upload/download/total, credits |
| Quota enforcement | Met — firewall rules with manifest ownership and recovery |
| Notifications | Met — events + in-app alerts |
| Temporary access | Met — credits with expiry |
| Extension connects | Met — native host with origin + allow-list validation |
| Site detection and policies | Met — hostname-keyed, synced to the service |
| Site blocking at quota | Met — DNR redirect/block, temp access |
| Truthful adapter reporting | Met — enforced/best_effort/unsupported with verification |
| Recovery | Met — stop/start/`recover` remove owned rules |
| Documentation | Met — this directory plus README |
