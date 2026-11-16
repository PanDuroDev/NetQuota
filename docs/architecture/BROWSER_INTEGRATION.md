# Browser integration

## Pieces

```
content script ──runtime messages──► service worker ──connectNative──► netquota-native-host ──pipe──► NetQuota service
      │                                    │
      │                                    └── declarativeNetRequest rules (block/redirect)
      └── PerformanceObserver + video adapters
```

The content script is the only code that touches page DOM. It can report usage,
read settings, and run video adapters. It cannot create blocking rules and cannot
reach native APIs.

## Native messaging

Host name `com.netquota.host`, registered per user under
`HKCU\Software\Google\Chrome\NativeMessagingHosts\com.netquota.host`.

- `register-native-host.ps1` writes the manifest with an `allowed_origins` entry
  for the unpacked extension id and installs it for the current user only.
- The host validates `argv[1]` as `chrome-extension://<32 chars>/`, then checks
  the id against `%NETQUOTA_DATA_DIR%\extension-id.txt`. A mismatch exits without
  reading stdin.
- Messages must be valid `Request` JSON and are re-serialized by the Rust
  protocol parser; anything malformed returns `Error{BadRequest}`.
- 1 MB ceiling in both directions.
- Chrome native messaging framing is a 4-byte little-endian length prefix, the
  same framing as the pipe.

Connection health: the worker pings on start, reconnects with exponential backoff
(2 s → 60 s) plus a one-minute alarm, and sets a `!` badge while disconnected.
The popup states clearly that browser rules still work but per-app usage and
profile changes need the service.

## Site identification and measurement

Sites are keyed by hostname (`new URL(tab.url).hostname`), never by IP or CDN.
Blocking one site therefore cannot block unrelated sites that share a CDN.

Byte measurement:

- `PerformanceResourceTiming.transferSize` is summed when non-zero.
- Cross-origin responses without `Timing-Allow-Origin` report `transferSize = 0`.
  Media on YouTube/TikTok is served from separate origins, so this path yields
  almost nothing.
- To keep quotas usable, playing media adds an explicit **bitrate model**:
  96 kbps at ≤144p, 160 at ≤240p, 320 at ≤360p, 560 at ≤480p, 1.2 Mbps at
  ≤720p, 2.4 at ≤1080p, 6 at ≤2160p, scaled by playing seconds while the tab is
  visible.

Everything the extension produces is flagged `estimated: true` and displayed as
"estimated" in both UIs. It is a budget, not a meter. The service's per-process
ETW numbers remain the measured source of truth for the browser as an
application.

## Quotas and blocking

- A site limit is stored in the service (`site_policies`) and mirrored locally so
  enforcement survives a disconnected service.
- When the day's ledger reaches the limit, the worker creates two dynamic rules:
  a `main_frame` redirect to `blocked.html?host=…`, and a block for every other
  resource type for that host.
- Rule ids are derived from the host hash, so re-applying is idempotent.
- Temporary access writes an expiry, removes the rules, and schedules
  `chrome.alarms` to re-apply the block after five minutes. The quota is not
  reset.
- The blocked page explains the reason and offers "Allow 5 minutes".

Known limitation: a site can serve content from a different hostname. Blocking
`youtube.com` does not block `googlevideo.com` media already in flight, and
blocking those CDNs would break unrelated sites. Per-site quotas are therefore
approximate at the network level, which is why the service-level quota remains
the hard control.

## Icons

`tab.favIconUrl` is provided by Chrome for the active tabs and cached in
`chrome.storage.local` as a data URL, with a generated fallback. No external icon
service is contacted. Desktop application icons are extracted from the executable
by the service (`SHGetFileInfo` + `GetDIBits` → PNG) and served to the desktop
app from the local cache.

## Permissions

`storage`, `tabs`, `alarms`, `notifications`, `declarativeNetRequest`,
`nativeMessaging`, host permission `<all_urls>` for content scripts. No
`webRequest`, no `debugger`, no `cookies`, no `history`, no remote code.
