# Testing

## Running

```powershell
cargo test --workspace          # Rust unit + integration tests
cargo test -p netquota-monitor  # one crate
cd apps/chrome-extension; npm run typecheck
```

Tests that touch the machine are written to be safe on a normal desktop: they
read the interface table, query the current process, and use in-memory SQLite or
temp files. No test downloads files from the internet, opens public URLs, or
leaves firewall rules behind.

## What is covered

| Area | Tests |
| --- | --- |
| Units, periods, quota evaluation | `crates/core` — boundary at `used >= limit`, direction-specific exhaustion, credits, DST-safe windows, week starts Monday |
| Protocol | framing, oversize frames, invalid hosts, request validation |
| Storage | migrations, upsert semantics, hourly aggregation, series queries, backup |
| Monitor | real interface read, loopback/filter exclusion, process identity, PID creation time, unknown attribution, monotonic counters across polls |
| Quota engine | transitions only once per window, pause overrides, background blocking, recovery on new window |
| Enforcement | rule naming/idempotency, dry-run round trip, foreign rule untouched, path validation |
| IPC | server/client round trip over the real named pipe |
| Native host | origin rejection, allow-list, oversize message, EOF |
| Service runtime | extension-id rejection, policy persistence, credit without policy, emergency restore, shutdown event |

## Scenario tests that need an elevated session

These cannot run in CI without an administrator, so they are documented and
scripted instead of faked:

1. Install the service, start it, confirm `netquota-cli status` shows
   `monitoring: running`.
2. Start a local TCP server and client (`python -m http.server` plus a loopback
   download) and confirm the per-process row grows by roughly the file size.
3. Quota exhaustion: set a 1 MB daily limit for the test executable, transfer
   more, observe the event, the block rule, and the app staying alive.
4. Temporary access: grant 5 minutes, confirm traffic resumes, then stops again.
5. Service restart while blocked: stop the service, confirm rules are removed,
   start it, confirm the state is restored from SQLite.
6. `netquota-cli recover` with the service stopped: rules removed from the
   manifest alone.

Expected overshoot: up to one poll interval (1 s) of traffic plus the firewall
apply latency before the block takes effect. The UI reports the measured value,
including the overshoot, and never rewrites history to look exact.

## Extension checks

Manual, because they need a real Chrome profile:

1. Load unpacked, confirm the badge is `!` until the service is running.
2. Visit a site, watch its row appear in the popup with "estimated" labelling.
3. Set a 5 MB limit, load media until it is reached, confirm the redirect page,
   then "Allow 5 minutes" and reload.
4. Disconnect the service: browser rules must keep working and the popup must say
   the desktop service is offline.
5. YouTube: select 144p, confirm the status is `enforced` only when the player
   reports a level at or below 144p.
6. TikTok: confirm quality is reported `unsupported` and off-screen videos pause.

## Performance

Monitoring cost was measured by leaving the service running and checking CPU and
working set in Task Manager: one ETW session, 1 Hz polling, 5 s database flush.
The service is single-threaded for state, with one thread per pipe connection, so
idle cost is dominated by ETW buffering rather than by application work.
