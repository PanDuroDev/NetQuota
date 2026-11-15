//! Local SQLite storage. Only metadata and aggregate counters are stored:
//! no page contents, no hostnames beyond the site level, no packet payloads.
//!
//! Writes are aggregated by the caller (the service flushes every 15 s), so the
//! database sees a handful of transactions per minute rather than per packet.

use std::path::Path;

use anyhow::{Context, Result};
use netquota_core::identity::AppIdentity;
use netquota_core::policy::AppPolicy;
use netquota_core::quota::Usage;
use netquota_core::rules::Rule;
use rusqlite::{params, Connection, OptionalExtension};

pub const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone, PartialEq)]
pub struct StoredApp {
    pub identity: AppIdentity,
    pub category: String,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
    pub icon_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredSite {
    pub host: String,
    pub down: u64,
    pub up: u64,
    pub active_secs: u64,
    pub last_seen_ms: i64,
    pub limit_bytes: Option<u64>,
    pub blocked: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredEvent {
    pub id: i64,
    pub ts_ms: i64,
    pub kind: String,
    pub subject: String,
    pub message: String,
    pub bytes: Option<u64>,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<()> {
        let current: i64 =
            self.conn
                .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if current < 1 {
            self.conn.execute_batch(
                r#"
                BEGIN;
                CREATE TABLE apps (
                    app_key       TEXT PRIMARY KEY,
                    exe_name      TEXT NOT NULL,
                    path          TEXT,
                    publisher     TEXT,
                    services      TEXT NOT NULL DEFAULT '',
                    category      TEXT NOT NULL DEFAULT 'other',
                    first_seen_ms INTEGER NOT NULL,
                    last_seen_ms  INTEGER NOT NULL,
                    icon_path     TEXT
                );
                CREATE TABLE app_policies (
                    app_key     TEXT PRIMARY KEY,
                    policy_json TEXT NOT NULL,
                    updated_ms  INTEGER NOT NULL
                );
                CREATE TABLE usage_hourly (
                    app_key TEXT NOT NULL,
                    hour_ms INTEGER NOT NULL,
                    down    INTEGER NOT NULL DEFAULT 0,
                    up      INTEGER NOT NULL DEFAULT 0,
                    PRIMARY KEY (app_key, hour_ms)
                ) WITHOUT ROWID;
                CREATE INDEX idx_usage_hour ON usage_hourly(hour_ms);
                CREATE TABLE sites (
                    host         TEXT PRIMARY KEY,
                    down         INTEGER NOT NULL DEFAULT 0,
                    up           INTEGER NOT NULL DEFAULT 0,
                    active_secs  INTEGER NOT NULL DEFAULT 0,
                    last_seen_ms INTEGER NOT NULL
                );
                CREATE TABLE site_policies (
                    host        TEXT PRIMARY KEY,
                    limit_bytes INTEGER,
                    blocked     INTEGER NOT NULL DEFAULT 0,
                    updated_ms  INTEGER NOT NULL
                );
                CREATE TABLE events (
                    id      INTEGER PRIMARY KEY AUTOINCREMENT,
                    ts_ms   INTEGER NOT NULL,
                    kind    TEXT NOT NULL,
                    subject TEXT NOT NULL DEFAULT '',
                    message TEXT NOT NULL DEFAULT '',
                    bytes   INTEGER
                );
                CREATE INDEX idx_events_ts ON events(ts_ms DESC);
                CREATE TABLE rules (
                    id         TEXT PRIMARY KEY,
                    json       TEXT NOT NULL,
                    updated_ms INTEGER NOT NULL
                );
                CREATE TABLE settings (
                    key   TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );
                PRAGMA user_version = 1;
                COMMIT;
                "#,
            )?;
        }
        Ok(())
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self.conn.query_row("PRAGMA user_version", [], |row| row.get(0))?)
    }

    pub fn upsert_app(&self, app: &AppIdentity, category: &str, now_ms: i64) -> Result<()> {
        let services = app.services.join(",");
        self.conn.execute(
            r#"
            INSERT INTO apps (app_key, exe_name, path, publisher, services, category, first_seen_ms, last_seen_ms)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
            ON CONFLICT(app_key) DO UPDATE SET
                exe_name     = excluded.exe_name,
                path         = excluded.path,
                publisher    = COALESCE(excluded.publisher, apps.publisher),
                services     = excluded.services,
                last_seen_ms = excluded.last_seen_ms
            "#,
            params![
                app.key,
                app.exe_name,
                app.path,
                app.publisher,
                services,
                category,
                now_ms
            ],
        )?;
        Ok(())
    }

    pub fn set_icon(&self, app_key: &str, icon_path: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE apps SET icon_path = ?2 WHERE app_key = ?1",
            params![app_key, icon_path],
        )?;
        Ok(())
    }

    pub fn list_apps(&self) -> Result<Vec<StoredApp>> {
        let mut stmt = self.conn.prepare(
            "SELECT app_key, exe_name, path, publisher, services, category, first_seen_ms, last_seen_ms, icon_path
             FROM apps ORDER BY exe_name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |row| {
            let services: String = row.get(4)?;
            Ok(StoredApp {
                identity: AppIdentity {
                    key: row.get(0)?,
                    exe_name: row.get(1)?,
                    path: row.get(2)?,
                    publisher: row.get(3)?,
                    services: services
                        .split(',')
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect(),
                },
                category: row.get(5)?,
                first_seen_ms: row.get(6)?,
                last_seen_ms: row.get(7)?,
                icon_path: row.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_policy(&self, app_key: &str) -> Result<Option<AppPolicy>> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT policy_json FROM app_policies WHERE app_key = ?1",
                params![app_key],
                |row| row.get(0),
            )
            .optional()?;
        match json {
            Some(json) => Ok(Some(serde_json::from_str(&json).context("corrupt policy")?)),
            None => Ok(None),
        }
    }

    pub fn put_policy(&self, policy: &AppPolicy, now_ms: i64) -> Result<()> {
        let json = serde_json::to_string(policy)?;
        self.conn.execute(
            r#"
            INSERT INTO app_policies (app_key, policy_json, updated_ms)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(app_key) DO UPDATE SET policy_json = excluded.policy_json, updated_ms = excluded.updated_ms
            "#,
            params![policy.app_key, json, now_ms],
        )?;
        Ok(())
    }

    pub fn all_policies(&self) -> Result<Vec<AppPolicy>> {
        let mut stmt = self
            .conn
            .prepare("SELECT policy_json FROM app_policies")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(serde_json::from_str(&row?).context("corrupt policy")?);
        }
        Ok(out)
    }

    /// Add pre-aggregated counters. Callers flush periodically, not per packet.
    pub fn add_usage(&mut self, entries: &[(String, i64, u64, u64)]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                r#"
                INSERT INTO usage_hourly (app_key, hour_ms, down, up)
                VALUES (?1, ?2, ?3, ?4)
                ON CONFLICT(app_key, hour_ms) DO UPDATE SET
                    down = down + excluded.down,
                    up   = up   + excluded.up
                "#,
            )?;
            for (key, hour, down, up) in entries {
                if *down == 0 && *up == 0 {
                    continue;
                }
                stmt.execute(params![key, hour, *down as i64, *up as i64])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn usage_for_app(&self, app_key: &str, from_ms: i64, to_ms: i64) -> Result<Usage> {
        let (down, up) = self.conn.query_row(
            "SELECT COALESCE(SUM(down),0), COALESCE(SUM(up),0) FROM usage_hourly
             WHERE app_key = ?1 AND hour_ms >= ?2 AND hour_ms < ?3",
            params![app_key, from_ms, to_ms],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )?;
        Ok(Usage {
            down: down.max(0) as u64,
            up: up.max(0) as u64,
        })
    }

    pub fn usage_totals(&self, from_ms: i64, to_ms: i64) -> Result<Usage> {
        let (down, up) = self.conn.query_row(
            "SELECT COALESCE(SUM(down),0), COALESCE(SUM(up),0) FROM usage_hourly
             WHERE hour_ms >= ?1 AND hour_ms < ?2",
            params![from_ms, to_ms],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )?;
        Ok(Usage {
            down: down.max(0) as u64,
            up: up.max(0) as u64,
        })
    }

    pub fn usage_top(&self, from_ms: i64, to_ms: i64, limit: u32) -> Result<Vec<(String, Usage)>> {
        let mut stmt = self.conn.prepare(
            "SELECT app_key, SUM(down), SUM(up) FROM usage_hourly
             WHERE hour_ms >= ?1 AND hour_ms < ?2
             GROUP BY app_key ORDER BY (SUM(down) + SUM(up)) DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![from_ms, to_ms, limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                Usage {
                    down: row.get::<_, i64>(1)?.max(0) as u64,
                    up: row.get::<_, i64>(2)?.max(0) as u64,
                },
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Hourly series for charts, summed by hour bucket.
    pub fn usage_series(&self, from_ms: i64, to_ms: i64) -> Result<Vec<(i64, Usage)>> {
        let mut stmt = self.conn.prepare(
            "SELECT hour_ms, SUM(down), SUM(up) FROM usage_hourly
             WHERE hour_ms >= ?1 AND hour_ms < ?2
             GROUP BY hour_ms ORDER BY hour_ms",
        )?;
        let rows = stmt.query_map(params![from_ms, to_ms], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                Usage {
                    down: row.get::<_, i64>(1)?.max(0) as u64,
                    up: row.get::<_, i64>(2)?.max(0) as u64,
                },
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn add_site_usage(
        &self,
        host: &str,
        down: u64,
        up: u64,
        active_secs: u64,
        now_ms: i64,
    ) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO sites (host, down, up, active_secs, last_seen_ms)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(host) DO UPDATE SET
                down = down + excluded.down,
                up = up + excluded.up,
                active_secs = active_secs + excluded.active_secs,
                last_seen_ms = excluded.last_seen_ms
            "#,
            params![host, down as i64, up as i64, active_secs as i64, now_ms],
        )?;
        Ok(())
    }

    pub fn list_sites(&self) -> Result<Vec<StoredSite>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.host, s.down, s.up, s.active_secs, s.last_seen_ms, p.limit_bytes,
                    COALESCE(p.blocked,0)
             FROM sites s LEFT JOIN site_policies p ON p.host = s.host
             ORDER BY (s.down + s.up) DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(StoredSite {
                host: row.get(0)?,
                down: row.get::<_, i64>(1)?.max(0) as u64,
                up: row.get::<_, i64>(2)?.max(0) as u64,
                active_secs: row.get::<_, i64>(3)?.max(0) as u64,
                last_seen_ms: row.get(4)?,
                limit_bytes: row.get::<_, Option<i64>>(5)?.map(|v| v.max(0) as u64),
                blocked: row.get::<_, i64>(6)? != 0,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn set_site_policy(
        &self,
        host: &str,
        limit_bytes: Option<u64>,
        blocked: bool,
        now_ms: i64,
    ) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO site_policies (host, limit_bytes, blocked, updated_ms)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(host) DO UPDATE SET
                limit_bytes = excluded.limit_bytes,
                blocked = excluded.blocked,
                updated_ms = excluded.updated_ms
            "#,
            params![host, limit_bytes.map(|v| v as i64), blocked as i64, now_ms],
        )?;
        Ok(())
    }

    pub fn site_limit(&self, host: &str) -> Result<Option<(Option<u64>, bool)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT limit_bytes, blocked FROM site_policies WHERE host = ?1",
                params![host],
                |row| {
                    Ok((
                        row.get::<_, Option<i64>>(0)?.map(|v| v.max(0) as u64),
                        row.get::<_, i64>(1)? != 0,
                    ))
                },
            )
            .optional()?)
    }

    /// Bounded delete of expired raw history.
    pub fn prune_usage(&self, older_than_ms: i64) -> Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM usage_hourly WHERE hour_ms < ?1",
            params![older_than_ms],
        )?)
    }

    pub fn add_event(
        &self,
        ts_ms: i64,
        kind: &str,
        subject: &str,
        message: &str,
        bytes: Option<u64>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO events (ts_ms, kind, subject, message, bytes) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![ts_ms, kind, subject, message, bytes.map(|b| b as i64)],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn events(&self, limit: u32) -> Result<Vec<StoredEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, ts_ms, kind, subject, message, bytes FROM events ORDER BY ts_ms DESC, id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |row| {
            Ok(StoredEvent {
                id: row.get(0)?,
                ts_ms: row.get(1)?,
                kind: row.get(2)?,
                subject: row.get(3)?,
                message: row.get(4)?,
                bytes: row.get::<_, Option<i64>>(5)?.map(|v| v.max(0) as u64),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn put_rule(&self, rule: &Rule, now_ms: i64) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO rules (id, json, updated_ms) VALUES (?1, ?2, ?3)
            ON CONFLICT(id) DO UPDATE SET json = excluded.json, updated_ms = excluded.updated_ms
            "#,
            params![rule.id, serde_json::to_string(rule)?, now_ms],
        )?;
        Ok(())
    }

    pub fn all_rules(&self) -> Result<Vec<Rule>> {
        let mut stmt = self.conn.prepare("SELECT json FROM rules ORDER BY id")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(serde_json::from_str(&row?).context("corrupt rule")?);
        }
        Ok(out)
    }

    pub fn delete_rule(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM rules WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// File-level backup using SQLite's own backup API.
    pub fn backup_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        self.conn
            .backup(rusqlite::DatabaseName::Main, path, None)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::open_in_memory().expect("in-memory store")
    }

    fn app(path: &str, exe: &str) -> AppIdentity {
        AppIdentity::new(Some(path), exe, vec![])
    }

    #[test]
    fn migrations_are_idempotent() {
        let s = store();
        assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn apps_are_upserted_not_duplicated() {
        let s = store();
        let a = app(r"C:\a\chrome.exe", "chrome.exe");
        s.upsert_app(&a, "browser", 1).unwrap();
        s.upsert_app(&a, "browser", 2).unwrap();
        let apps = s.list_apps().unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].last_seen_ms, 2);
    }

    #[test]
    fn usage_is_summed_into_hour_buckets() {
        let mut s = store();
        s.add_usage(&[("k".into(), 100, 10, 1), ("k".into(), 100, 5, 2)])
            .unwrap();
        let u = s.usage_for_app("k", 0, 200).unwrap();
        assert_eq!(u, Usage { down: 15, up: 3 });
        assert_eq!(s.usage_series(0, 200).unwrap().len(), 1);
    }

    #[test]
    fn usage_range_excludes_outside_buckets() {
        let mut s = store();
        s.add_usage(&[
            ("k".into(), 100, 10, 0),
            ("k".into(), 200, 20, 0),
            ("k".into(), 300, 30, 0),
        ])
        .unwrap();
        assert_eq!(
            s.usage_for_app("k", 100, 300).unwrap(),
            Usage { down: 30, up: 0 }
        );
        assert_eq!(
            s.usage_totals(100, 400).unwrap(),
            Usage { down: 60, up: 0 }
        );
    }

    #[test]
    fn top_consumers_are_ordered() {
        let mut s = store();
        s.add_usage(&[("a".into(), 100, 10, 0), ("b".into(), 100, 99, 0)])
            .unwrap();
        let top = s.usage_top(0, 200, 10).unwrap();
        assert_eq!(top[0].0, "b");
    }

    #[test]
    fn policy_round_trips() {
        let s = store();
        s.upsert_app(&app(r"C:\a\app.exe", "app.exe"), "other", 1)
            .unwrap();
        let mut p = AppPolicy::new(r"c:\a\app.exe");
        p.limit.total = Some(1_000);
        s.put_policy(&p, 1).unwrap();
        assert_eq!(s.get_policy(&p.app_key).unwrap().unwrap(), p);
        assert_eq!(s.all_policies().unwrap().len(), 1);
    }

    #[test]
    fn policy_can_exist_before_the_app_is_seen() {
        // A user may set a quota for something that has not run yet.
        let s = store();
        let mut p = AppPolicy::new(r"c:\games\launcher.exe");
        p.limit.total = Some(5_000);
        s.put_policy(&p, 1).unwrap();
        assert_eq!(s.get_policy(&p.app_key).unwrap().unwrap(), p);
    }

    #[test]
    fn site_usage_accumulates_and_limits_apply() {
        let s = store();
        s.add_site_usage("youtube.com", 100, 5, 10, 1).unwrap();
        s.add_site_usage("youtube.com", 50, 5, 10, 2).unwrap();
        s.set_site_policy("youtube.com", Some(1_000), true, 3)
            .unwrap();
        let sites = s.list_sites().unwrap();
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].down, 150);
        assert_eq!(sites[0].limit_bytes, Some(1_000));
        assert!(sites[0].blocked);
    }

    #[test]
    fn events_are_returned_newest_first_and_bounded() {
        let s = store();
        for i in 0..5 {
            s.add_event(i, "info", "app", "msg", None).unwrap();
        }
        let events = s.events(3).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].ts_ms, 4);
    }

    #[test]
    fn prune_removes_old_buckets_only() {
        let mut s = store();
        s.add_usage(&[("a".into(), 100, 1, 0), ("a".into(), 500, 2, 0)])
            .unwrap();
        assert_eq!(s.prune_usage(200).unwrap(), 1);
        assert_eq!(
            s.usage_totals(0, 1000).unwrap(),
            Usage { down: 2, up: 0 }
        );
    }

    #[test]
    fn settings_and_rules_round_trip() {
        let s = store();
        s.set_setting("theme", "dark").unwrap();
        assert_eq!(s.get_setting("theme").unwrap().as_deref(), Some("dark"));
        let r = netquota_core::rules::rule(
            "r1",
            netquota_core::rules::Trigger::MeteredConnection,
            netquota_core::rules::Action::ActivateProfile {
                profile: netquota_core::ProfileKind::StrictDataSaver,
            },
        );
        s.put_rule(&r, 1).unwrap();
        assert_eq!(s.all_rules().unwrap(), vec![r]);
    }

    #[test]
    fn policies_outlive_app_rows() {
        // Discovery is transient; a quota is user intent and must survive a
        // pruned app row.
        let s = store();
        s.upsert_app(&app(r"C:\a\app.exe", "app.exe"), "other", 1)
            .unwrap();
        s.put_policy(&AppPolicy::new(r"c:\a\app.exe"), 1).unwrap();
        s.conn
            .execute("DELETE FROM apps WHERE app_key = ?1", params![r"c:\a\app.exe"])
            .unwrap();
        assert_eq!(s.all_policies().unwrap().len(), 1);
    }
}
