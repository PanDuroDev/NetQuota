# NetQuota Chrome extension

MV3 extension. Build with `scripts/build-extension.ps1` (or `npm install && npx
tsc -p tsconfig.json` in this folder) and load this directory unpacked.

## What it does

- Tracks estimated per-site usage in a local daily ledger.
- Blocks a site at its quota with dynamic declarativeNetRequest rules and a
  blocked page that offers five minutes of temporary access.
- Reports site totals to the NetQuota service through `com.netquota.host`, and
  keeps enforcing locally while the service is disconnected (badge shows `!`).
- Runs video adapters and reports their real support level, never an optimistic
  one.

## Files

```
manifest.json          MV3 manifest (no remote code, no webRequest)
src/background.ts      service worker: native bridge, ledger, DNR, notifications
src/content.ts         measurement + adapter registry (YouTube, TikTok, generic)
src/popup.ts|html      per-site and global controls
src/blocked.html|ts    blocked page with temporary access
src/shared/types.ts    type-only mirror of the Rust protocol
```

`dist/` is generated and not committed. The manifest points at `dist/*.js`, so
build before loading.

## Honest limitations

- Cross-origin media does not expose byte counts; usage is an estimate.
- Blocking is per hostname, so a site's CDN traffic on another domain is not
  blocked. Service-level quotas remain the hard control.
- TikTok has no reliable quality API; the adapter reports `unsupported` for
  quality and restricts autoplay/preload/off-screen playback instead.
