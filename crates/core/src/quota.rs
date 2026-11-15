use serde::{Deserialize, Serialize};

/// Byte totals for one application or one website in a period.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub down: u64,
    pub up: u64,
}

impl Usage {
    pub fn total(self) -> u64 {
        self.down.saturating_add(self.up)
    }

    pub fn add(&mut self, down: u64, up: u64) {
        self.down = self.down.saturating_add(down);
        self.up = self.up.saturating_add(up);
    }
}

/// Independent limits. `None` means unlimited for that dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limit {
    pub total: Option<u64>,
    pub download: Option<u64>,
    pub upload: Option<u64>,
}

impl Limit {
    pub const UNLIMITED: Self = Self {
        total: None,
        download: None,
        upload: None,
    };

    pub fn is_unlimited(&self) -> bool {
        self.total.is_none() && self.download.is_none() && self.upload.is_none()
    }
}

impl Default for Limit {
    fn default() -> Self {
        Self::UNLIMITED
    }
}

/// Temporary or permanent extra allowance. Expired credits are ignored at
/// evaluation time but are kept for history until the period resets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credit {
    pub extra_total: u64,
    pub extra_down: u64,
    pub extra_up: u64,
    pub until_ms: Option<i64>,
    pub reason: String,
}

impl Credit {
    pub fn active_at(&self, now_ms: i64) -> bool {
        self.until_ms.map_or(true, |until| now_ms < until)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Download,
    Upload,
    /// A combined limit was exceeded, so both directions are restricted.
    Both,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evaluation {
    pub used: Usage,
    pub limit: Limit,
    pub remaining_total: Option<i64>,
    pub remaining_down: Option<i64>,
    pub remaining_up: Option<i64>,
    /// `None` means the application is allowed to use the network.
    pub exhausted: Option<Direction>,
    pub warn: bool,
    /// Highest utilisation across configured limits, clamped to 0.0..=1.0.
    pub ratio: Option<f64>,
}

impl Evaluation {
    pub fn is_allowed(&self) -> bool {
        self.exhausted.is_none()
    }
}

fn plus_extra(limit: Option<u64>, extra: u64) -> Option<u64> {
    limit.map(|l| l.saturating_add(extra))
}

fn ratio(used: u64, limit: Option<u64>) -> Option<f64> {
    let limit = limit?;
    if limit == 0 {
        return Some(1.0);
    }
    Some((used as f64 / limit as f64).clamp(0.0, 1.0))
}

fn remaining(used: u64, limit: Option<u64>) -> Option<i64> {
    limit.map(|l| l as i64 - used as i64)
}

/// Pure quota decision. Order of checks matters: a combined limit takes
/// precedence because exceeding it restricts the whole application.
pub fn evaluate(
    used: Usage,
    limit: Limit,
    credit: Option<&Credit>,
    now_ms: i64,
    warn_ratio: f64,
) -> Evaluation {
    let (extra_total, extra_down, extra_up) = match credit.filter(|c| c.active_at(now_ms)) {
        Some(c) => (c.extra_total, c.extra_down, c.extra_up),
        None => (0, 0, 0),
    };
    let eff_total = plus_extra(limit.total, extra_total);
    let eff_down = plus_extra(limit.download, extra_down);
    let eff_up = plus_extra(limit.upload, extra_up);

    let total_exceeded = eff_total.is_some_and(|l| used.total() >= l);
    let down_exceeded = eff_down.is_some_and(|l| used.down >= l);
    let up_exceeded = eff_up.is_some_and(|l| used.up >= l);

    let exhausted = if total_exceeded {
        Some(Direction::Both)
    } else {
        match (down_exceeded, up_exceeded) {
            (true, true) => Some(Direction::Both),
            (true, false) => Some(Direction::Download),
            (false, true) => Some(Direction::Upload),
            (false, false) => None,
        }
    };

    let ratio = [
        ratio(used.total(), eff_total),
        ratio(used.down, eff_down),
        ratio(used.up, eff_up),
    ]
    .into_iter()
    .flatten()
    .reduce(f64::max);

    Evaluation {
        used,
        limit,
        remaining_total: remaining(used.total(), eff_total),
        remaining_down: remaining(used.down, eff_down),
        remaining_up: remaining(used.up, eff_up),
        exhausted,
        warn: exhausted.is_none() && ratio.is_some_and(|r| r >= warn_ratio),
        ratio,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000_000;

    fn limit(total: u64) -> Limit {
        Limit {
            total: Some(total),
            ..Limit::UNLIMITED
        }
    }

    #[test]
    fn allows_usage_below_limit() {
        let e = evaluate(
            Usage { down: 400, up: 100 },
            limit(1000),
            None,
            NOW,
            0.8,
        );
        assert!(e.is_allowed());
        assert_eq!(e.remaining_total, Some(500));
        assert!(!e.warn);
    }

    #[test]
    fn blocks_exactly_at_the_limit_boundary() {
        let e = evaluate(
            Usage { down: 600, up: 400 },
            limit(1000),
            None,
            NOW,
            0.8,
        );
        assert_eq!(e.exhausted, Some(Direction::Both));
        assert_eq!(e.remaining_total, Some(0));
    }

    #[test]
    fn one_byte_below_limit_is_allowed_but_warns() {
        let e = evaluate(
            Usage { down: 999, up: 0 },
            limit(1000),
            None,
            NOW,
            0.8,
        );
        assert!(e.is_allowed());
        assert!(e.warn);
    }

    #[test]
    fn direction_limits_block_independently() {
        let l = Limit {
            total: None,
            download: Some(100),
            upload: Some(50),
        };
        let down = evaluate(Usage { down: 100, up: 10 }, l, None, NOW, 0.8);
        assert_eq!(down.exhausted, Some(Direction::Download));
        let up = evaluate(Usage { down: 10, up: 50 }, l, None, NOW, 0.8);
        assert_eq!(up.exhausted, Some(Direction::Upload));
        let both = evaluate(Usage { down: 100, up: 50 }, l, None, NOW, 0.8);
        assert_eq!(both.exhausted, Some(Direction::Both));
    }

    #[test]
    fn zero_limit_blocks_immediately() {
        let e = evaluate(Usage::default(), limit(0), None, NOW, 0.8);
        assert_eq!(e.exhausted, Some(Direction::Both));
    }

    #[test]
    fn credit_extends_the_allowance_within_its_window() {
        let credit = Credit {
            extra_total: 500,
            extra_down: 0,
            extra_up: 0,
            until_ms: Some(NOW + 1000),
            reason: "temporary access".into(),
        };
        let allowed = evaluate(
            Usage { down: 1200, up: 0 },
            limit(1000),
            Some(&credit),
            NOW,
            0.8,
        );
        assert!(allowed.is_allowed());
        let expired = evaluate(
            Usage { down: 1200, up: 0 },
            limit(1000),
            Some(&credit),
            NOW + 1000,
            0.8,
        );
        assert_eq!(expired.exhausted, Some(Direction::Both));
    }

    #[test]
    fn expired_credit_does_not_apply() {
        let credit = Credit {
            extra_total: 10_000,
            extra_down: 0,
            extra_up: 0,
            until_ms: Some(NOW - 1),
            reason: "expired".into(),
        };
        let e = evaluate(
            Usage { down: 2000, up: 0 },
            limit(1000),
            Some(&credit),
            NOW,
            0.8,
        );
        assert_eq!(e.exhausted, Some(Direction::Both));
    }

    #[test]
    fn unlimited_never_blocks() {
        let e = evaluate(
            Usage {
                down: u64::MAX / 4,
                up: u64::MAX / 4,
            },
            Limit::UNLIMITED,
            None,
            NOW,
            0.8,
        );
        assert!(e.is_allowed());
        assert_eq!(e.ratio, None);
    }

    #[test]
    fn overshoot_is_reported_as_negative_remaining() {
        let e = evaluate(
            Usage { down: 1_500, up: 0 },
            limit(1_000),
            None,
            NOW,
            0.8,
        );
        assert_eq!(e.remaining_total, Some(-500));
    }
}
