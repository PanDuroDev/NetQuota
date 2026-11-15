use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};

/// A quota period. Windows-local time; periods reset at local midnight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeriodKind {
    Daily,
    Weekly,
    Monthly,
    /// Rolling window of `days` calendar days, anchored to local midnight.
    CustomDays { days: u32 },
}

impl PeriodKind {
    pub fn as_str(self) -> &'static str {
        match self {
            PeriodKind::Daily => "daily",
            PeriodKind::Weekly => "weekly",
            PeriodKind::Monthly => "monthly",
            PeriodKind::CustomDays { .. } => "custom",
        }
    }
}

impl Default for PeriodKind {
    fn default() -> Self {
        Self::Daily
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeriodWindow {
    pub start_ms: i64,
    pub end_ms: i64,
}

impl PeriodWindow {
    pub fn contains(&self, ts_ms: i64) -> bool {
        ts_ms >= self.start_ms && ts_ms < self.end_ms
    }
}

fn next_month(first_of_month: NaiveDate) -> NaiveDate {
    let (year, month) = if first_of_month.month() == 12 {
        (first_of_month.year() + 1, 1)
    } else {
        (first_of_month.year(), first_of_month.month() + 1)
    };
    NaiveDate::from_ymd_opt(year, month, 1).unwrap_or(first_of_month + Duration::days(31))
}

/// Local midnight in epoch milliseconds. DST transitions can make midnight
/// non-existent; the earliest valid local instant is used in that case.
fn local_midnight_ms(date: NaiveDate) -> i64 {
    for hour in [0, 1, 2] {
        let naive = date.and_hms_opt(hour, 0, 0).expect("valid time");
        if let Some(dt) = Local.from_local_datetime(&naive).earliest() {
            return dt.timestamp_millis();
        }
    }
    date.and_hms_opt(0, 0, 0)
        .expect("valid time")
        .and_utc()
        .timestamp_millis()
}

pub fn window_for(kind: PeriodKind, now: DateTime<Local>) -> PeriodWindow {
    let today = now.date_naive();
    let start_date = match kind {
        PeriodKind::Daily => today,
        PeriodKind::Weekly => {
            today - Duration::days(today.weekday().num_days_from_monday() as i64)
        }
        PeriodKind::Monthly => {
            NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today)
        }
        PeriodKind::CustomDays { days } => {
            today - Duration::days(days.max(1) as i64 - 1)
        }
    };
    let end_date = match kind {
        PeriodKind::Daily => start_date + Duration::days(1),
        PeriodKind::Weekly => start_date + Duration::days(7),
        PeriodKind::Monthly => next_month(start_date),
        PeriodKind::CustomDays { days } => start_date + Duration::days(days.max(1) as i64),
    };
    PeriodWindow {
        start_ms: local_midnight_ms(start_date),
        end_ms: local_midnight_ms(end_date),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, m: u32, d: u32, h: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(y, m, d, h, 30, 0)
            .single()
            .expect("valid local time")
    }

    #[test]
    fn daily_window_is_local_midnight_to_midnight() {
        let w = window_for(PeriodKind::Daily, at(2026, 3, 15, 13));
        assert_eq!(w.end_ms - w.start_ms, 24 * 60 * 60 * 1000);
        assert!(w.contains(w.start_ms));
        assert!(!w.contains(w.end_ms));
    }

    #[test]
    fn weekly_window_starts_monday() {
        // 2026-03-15 is a Sunday.
        let w = window_for(PeriodKind::Weekly, at(2026, 3, 15, 9));
        let start = Local.timestamp_millis_opt(w.start_ms).unwrap();
        assert_eq!(start.weekday(), chrono::Weekday::Mon);
        assert_eq!(start.day(), 9);
    }

    #[test]
    fn monthly_window_covers_month() {
        let w = window_for(PeriodKind::Monthly, at(2026, 12, 31, 23));
        let start = Local.timestamp_millis_opt(w.start_ms).unwrap();
        let end = Local.timestamp_millis_opt(w.end_ms).unwrap();
        assert_eq!((start.month(), start.day()), (12, 1));
        assert_eq!((end.year(), end.month(), end.day()), (2027, 1, 1));
    }

    #[test]
    fn custom_window_spans_requested_days() {
        let w = window_for(PeriodKind::CustomDays { days: 7 }, at(2026, 3, 15, 8));
        assert_eq!(w.end_ms - w.start_ms, 7 * 24 * 60 * 60 * 1000);
    }

    #[test]
    fn window_is_stable_across_the_day() {
        let morning = window_for(PeriodKind::Daily, at(2026, 3, 15, 1));
        let evening = window_for(PeriodKind::Daily, at(2026, 3, 15, 23));
        assert_eq!(morning, evening);
    }
}
