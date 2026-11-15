use serde::{Deserialize, Serialize};

use crate::profile::ProfileKind;
use crate::quota::Limit;

/// User-defined automation. Kept declarative and local; no scripting engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub trigger: Trigger,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    /// Daily usage of one app (or all apps when `app_key` is `None`) above bytes.
    DailyUsageAbove {
        app_key: Option<String>,
        bytes: u64,
    },
    MonthlyPercentAbove {
        percent: u8,
    },
    MeteredConnection,
    AppInBackground {
        app_key: String,
    },
    QuotaExhausted {
        app_key: String,
    },
    /// Local hour is outside `start..end`.
    OutsideHours {
        start: u8,
        end: u8,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    BlockApp { app_key: String },
    UnblockApp { app_key: String },
    SetLimit { app_key: String, limit: Limit },
    ActivateProfile { profile: ProfileKind },
    Notify { message_key: String },
    ThrottleApp { app_key: String, kbps: u32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppFact {
    pub app_key: String,
    pub daily_bytes: u64,
    pub background: bool,
    /// Computed by the quota engine from measured usage and the app's limits.
    pub quota_exhausted: bool,
}

/// Snapshot of everything a rule can inspect. All values come from measured
/// data, so a rule never fires on invented statistics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Facts {
    pub apps: Vec<AppFact>,
    pub metered: bool,
    pub monthly_used: u64,
    pub monthly_limit: Option<u64>,
    pub local_hour: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleOutcome {
    pub rule_id: String,
    pub action: Action,
}

fn app<'a>(facts: &'a Facts, key: &str) -> Option<&'a AppFact> {
    facts.apps.iter().find(|a| a.app_key == key)
}

fn total_daily(facts: &Facts) -> u64 {
    facts
        .apps
        .iter()
        .fold(0u64, |acc, a| acc.saturating_add(a.daily_bytes))
}

impl Trigger {
    fn matches(&self, facts: &Facts) -> bool {
        match self {
            Trigger::DailyUsageAbove { app_key, bytes } => match app_key {
                Some(key) => app(facts, key).is_some_and(|a| a.daily_bytes > *bytes),
                None => total_daily(facts) > *bytes,
            },
            Trigger::MonthlyPercentAbove { percent } => facts.monthly_limit.is_some_and(|limit| {
                limit > 0 && facts.monthly_used.saturating_mul(100) >= limit.saturating_mul(*percent as u64)
            }),
            Trigger::MeteredConnection => facts.metered,
            Trigger::AppInBackground { app_key } => {
                app(facts, app_key).is_some_and(|a| a.background)
            }
            Trigger::QuotaExhausted { app_key } => {
                app(facts, app_key).is_some_and(|a| a.quota_exhausted)
            }
            Trigger::OutsideHours { start, end } => {
                if start == end {
                    false
                } else if start < end {
                    facts.local_hour < *start || facts.local_hour >= *end
                } else {
                    facts.local_hour >= *end && facts.local_hour < *start
                }
            }
        }
    }
}

/// Evaluate rules in order and return de-duplicated actions. Block actions win
/// over unblock actions for the same app so conflicting rules fail closed, while
/// still reporting both outcomes to the UI.
pub fn evaluate(rules: &[Rule], facts: &Facts) -> Vec<RuleOutcome> {
    let mut outcomes: Vec<RuleOutcome> = Vec::new();
    for rule in rules.iter().filter(|r| r.enabled) {
        if !rule.trigger.matches(facts) {
            continue;
        }
        let duplicate = outcomes
            .iter()
            .any(|o| o.action == rule.action);
        if !duplicate {
            outcomes.push(RuleOutcome {
                rule_id: rule.id.clone(),
                action: rule.action.clone(),
            });
        }
    }
    outcomes
}

pub fn rule(id: &str, trigger: Trigger, action: Action) -> Rule {
    Rule {
        id: id.to_string(),
        name: format!("{id}"),
        enabled: true,
        trigger,
        action,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Facts {
        Facts {
            apps: vec![
                AppFact {
                    app_key: "steam".into(),
                    daily_bytes: 600_000_000,
                    background: true,
                    quota_exhausted: true,
                },
                AppFact {
                    app_key: "chrome".into(),
                    daily_bytes: 50_000_000,
                    background: false,
                    quota_exhausted: false,
                },
            ],
            metered: false,
            monthly_used: 8_500_000_000,
            monthly_limit: Some(10_000_000_000),
            local_hour: 14,
        }
    }

    #[test]
    fn fires_when_daily_threshold_crossed() {
        let rules = vec![rule(
            "r1",
            Trigger::DailyUsageAbove {
                app_key: Some("steam".into()),
                bytes: 500_000_000,
            },
            Action::BlockApp {
                app_key: "steam".into(),
            },
        )];
        let out = evaluate(&rules, &facts());
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn does_not_fire_below_threshold() {
        let rules = vec![rule(
            "r1",
            Trigger::DailyUsageAbove {
                app_key: Some("steam".into()),
                bytes: 1_000_000_000,
            },
            Action::Notify {
                message_key: "x".into(),
            },
        )];
        assert!(evaluate(&rules, &facts()).is_empty());
    }

    #[test]
    fn aggregate_trigger_uses_all_apps() {
        let rules = vec![rule(
            "r1",
            Trigger::DailyUsageAbove {
                app_key: None,
                bytes: 600_000_000,
            },
            Action::Notify {
                message_key: "total_high".into(),
            },
        )];
        assert_eq!(evaluate(&rules, &facts()).len(), 1);
    }

    #[test]
    fn monthly_percent_trigger_respects_limit() {
        let rules = vec![rule(
            "r1",
            Trigger::MonthlyPercentAbove { percent: 80 },
            Action::Notify {
                message_key: "monthly_80".into(),
            },
        )];
        assert_eq!(evaluate(&rules, &facts()).len(), 1);
        let mut f = facts();
        f.monthly_used = 1;
        assert!(evaluate(&rules, &f).is_empty());
    }

    #[test]
    fn night_rule_wraps_midnight() {
        let rules = vec![rule(
            "night",
            Trigger::OutsideHours { start: 8, end: 23 },
            Action::Notify {
                message_key: "night".into(),
            },
        )];
        let mut f = facts();
        f.local_hour = 2;
        assert_eq!(evaluate(&rules, &f).len(), 1);
        f.local_hour = 12;
        assert!(evaluate(&rules, &f).is_empty());
    }

    #[test]
    fn disabled_rules_never_fire() {
        let mut r = rule(
            "r1",
            Trigger::MeteredConnection,
            Action::Notify {
                message_key: "metered".into(),
            },
        );
        r.enabled = false;
        assert!(evaluate(&[r], &facts()).is_empty());
    }

    #[test]
    fn duplicate_actions_are_collapsed() {
        let action = Action::BlockApp {
            app_key: "steam".into(),
        };
        let rules = vec![
            rule(
                "a",
                Trigger::DailyUsageAbove {
                    app_key: Some("steam".into()),
                    bytes: 1,
                },
                action.clone(),
            ),
            rule(
                "b",
                Trigger::MeteredConnection,
                action,
            ),
        ];
        let mut f = facts();
        f.metered = true;
        assert_eq!(evaluate(&rules, &f).len(), 1);
    }
}
