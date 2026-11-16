//! Domain logic for NetQuota. Deliberately platform-neutral and pure so it can be
//! unit-tested on any machine and reused by the service, the desktop app and the CLI.

#![forbid(unsafe_code)]

pub mod categorize;
pub mod identity;
pub mod period;
pub mod policy;
pub mod profile;
pub mod protect;
pub mod quota;
pub mod rules;
pub mod units;

pub use identity::AppIdentity;
pub use period::{window_for, PeriodKind, PeriodWindow};
pub use policy::{AppPolicy, BackgroundPolicy, Enforcement};
pub use profile::{ProfileKind, ProfileSettings, VideoQuality};
pub use quota::{evaluate, Credit, Direction, Evaluation, Limit, Usage};
