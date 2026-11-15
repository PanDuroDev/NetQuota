# NetQuota desktop

Tauri 2 shell. The UI is plain TypeScript with no framework and no bundler, so a
build is `tsc` plus `cargo`.

```powershell
cd apps/desktop/ui; npm install; npx tsc -p tsconfig.json
cd ..\..\..
cargo run -p netquota-desktop
```

The window talks to the Windows service over the named pipe through the commands
in `src/main.rs`. When the service is absent the app falls back to interface
counters (`fallback_usage`), which need no administrator rights, and says so in a
banner instead of showing zeros as if they were real.

Commands: `health`, `snapshot`, `fallback_usage`, `apps`, `sites`, `events`,
`save_policy`, `set_paused`, `grant_credit`, `set_blocked`, `apply_profile`,
`set_site_policy`, `restore_all`, `app_icon`, `global_usage`.

`app_icon` only accepts an absolute `.exe` path and returns PNG bytes extracted by
`netquota-monitor::icons`; results are cached per session.

## Known limits

- Data saver quality is applied at the browser level by the extension; this window
  can change the global profile but cannot force a site's quality.
- The rules editor is not present yet; the policy engine is available in
  `netquota-core::rules` and is exercised by tests.
