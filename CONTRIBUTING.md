# Contributing

## Ground rules

1. Do not describe a feature as working until it has been exercised on Windows.
   Adapters, monitoring providers, and enforcement all have a "not supported"
   state; use it.
2. Never fabricate measurements. If attribution is incomplete, report
   `estimated` or `unknown`.
3. Never widen enforcement privileges. New privileged operations belong in the
   service behind a validated request, not in the UI or the extension.
4. Firewall code may only touch rules in the `NetQuota` group that are recorded
   in the ownership manifest.
5. Keep dependencies small. The project intentionally avoids a frontend
   framework, an HTTP client, and any cloud SDK.

## Development

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets
cd apps/chrome-extension; npm run typecheck
cd apps/desktop/ui; npx tsc -p tsconfig.json
```

Windows-specific crates are gated behind `#[cfg(windows)]`; keep `netquota-core`
and `netquota-quota-engine` platform-free so their tests run anywhere.

## Adding a video adapter

Add an object implementing `VideoAdapter` in the extension's content script
registry and verify it against the live site. Document, in
`docs/architecture/VIDEO_ADAPTERS.md`, which control you used and how you checked
that it took effect. An adapter that only detects a `<video>` element is not
support.

## Adding a monitoring provider

Implement the provider shape in `crates/monitor`, state in its doc comment what
traffic it sees, what it misses, and what privilege it needs. Then add it to the
supervisor behind a capability flag so the UI can show `degraded` instead of
silently losing data.

## Commits and pull requests

- One concern per commit; include the test you ran in the message when it is not
  obvious.
- Do not commit `target/`, `node_modules/`, `dist/`, databases, logs, or binaries.
- Pull requests are reviewed before merge. Do not merge your own without
  authorization.
