# Enforcement

NetQuota changes network policy, so every mechanism here is chosen for
reversibility first.

## Blocking an application

Implementation: Windows Firewall COM (`INetFwPolicy2` / `INetFwRule`), in
`crates/enforcement/src/firewall.rs`.

- One **outbound** rule per executable path, action `BLOCK`, protocol `ANY`,
  profiles `ALL`, edge traversal disabled.
- Rule name: `NetQuota\Block\<FNV-1a of lowercased path>`, group `NetQuota`.
  Hashing makes the operation idempotent and case-insensitive.
- The executable path is embedded in the rule description and in
  `firewall-manifest.json`, which lives next to the database.
- **Only names recorded in the manifest are ever removed.** `unblock` on an
  unknown name returns `false` without touching the firewall. Foreign and
  Microsoft rules are never edited, disabled, or deleted.
- Windows Firewall itself is never disabled.

The application keeps running. NetQuota blocks its traffic; it does not terminate
processes, because killing a process can lose the user's work and does not stop a
launcher from restarting it.

Why outbound only: an inbound rule for a desktop application would be a firewall
hole with no data-saving benefit. If a program listens on a port, blocking its
outbound traffic already prevents it from fetching anything.

### Recovery

1. Service stop: `Runtime::shutdown()` calls `restore_all()` before exiting.
2. Next start: `Runtime::open()` reads the manifest and removes leftover rules
   from a previous crash.
3. `netquota-cli recover`: sends `EmergencyRestore` to the service, or with no
   service running, loads the manifest and removes the rules directly.
4. Manual: Windows Firewall → Outbound rules → group `NetQuota` → disable/delete.
   The group exists precisely so a human can do this without guessing.

`EmergencyRestore` also clears the quota engine's blocked state, so a restored
app is not immediately blocked again on the next evaluation unless its policy
still says so.

### Validation before a rule is created

- Absolute path, `.exe`, no quotes or control characters.
- The path may be missing (recovery of a moved/removed executable is allowed).
- Protected identities (Windows Security, core networking services, `System32`
  binaries) return `Protection::Protected` / `RequiresConfirmation` from
  `netquota_core::protect`; the UI warns and the service refuses the ones marked
  `Protected`.

## Speed limits — what actually works

`AppPolicy` stores `upload_kbps` and `download_kbps`, and the UI shows them. The
enforcement crate reports their support honestly:

```rust
pub enum SpeedLimitSupport {
    UploadExperimental, // QoS policy, outbound only
    UnsupportedDownload, // needs a WFP shaper
}
```

Windows QoS (`ThrottleRateActionBitsPerSecond`, applied through a QoS policy or
group policy) throttles **outbound** traffic for a process. It is not a
bidirectional shaper:

- A download limit cannot be implemented with QoS. Setting one would be a lie.
- Upload throttling is available through the policy APIs but is experimental in
  this build: it is stored and displayed, and marked unenforced.

**Consequence:** in v0.1 the speed fields are policy metadata only. Neither
direction is enforced. True bidirectional shaping needs a Windows Filtering
Platform callout driver (signed, installable, with its own crash safety). That is
a later milestone; shipping an unsigned driver to claim the feature exists would
be worse than not shipping it.

## Background policies

`Enforcement::BlockBackground` plus `BackgroundPolicy` is evaluated by the quota
engine, which receives `background_blocked` only when the app is not the
foreground process **and** the configured condition matches (always, metered
network, non-Wi-Fi). `foreground_only` apps are allowed while focused.

`allowed_hours` is stored and validated (0–23, start ≤ end) but is not applied by
the v0.1 engine; the Rules page exposes it for a later phase.

## Notifications

The service records every transition in `events` and the desktop app shows an
in-app notification with the application name and icon. When the session allows
it, a Windows toast is raised through the desktop app rather than the service so
notification identity stays per-user.

Available actions on a quota alert: keep blocked, allow again, add 50/100/500 MB,
allow for 5/15/30 minutes, reset the current quota. Credits are `Credit` values
with an optional expiry, evaluated in `quota::evaluate`; they do not modify the
stored limit.

## Safety invariants

- No rule is created for a process NetQuota cannot resolve to an executable.
- No critical Windows service is blocked by a default or profile policy.
- Every applied rule is removable without the database, from the manifest alone.
- A crash leaves at most the rules already applied, and the next service start
  removes them before monitoring resumes.
