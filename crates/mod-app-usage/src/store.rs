//! SQLite 持久化：应用元数据、小时桶、会话与报表查询。
//!
//! 表结构由模块迁移创建；写入使用事务和 `ON CONFLICT ... DO UPDATE`，
//! 重复刷写只增加增量，不会覆盖历史总时长。

use crate::config::MODULE_ID;
use crate::report::{
    AppUsageSummary, CategoryUsage, DailyUsage, HourlyBreakdown, HourlyUsage, TrackedApp,
    UsageOverview,
};
use app_core::{CoreError, CoreResult, ModuleContext, ModuleMigration};
use chrono::{Local, NaiveDate};
use rusqlite::{params, OptionalExtension, Transaction};
use std::collections::BTreeMap;

const MAX_REPORT_DAYS: usize = 366;

/// 当前被跟踪应用的元数据快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppMeta {
    pub app_id: String,
    pub app_name: String,
    pub executable_path: Option<String>,
    pub window_title: Option<String>,
}

/// 一段连续前台会话，关闭时落盘。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClosedSession {
    pub app: AppMeta,
    pub started_at: i64,
    pub ended_at: i64,
    pub active_seconds: u64,
}

/// 待刷写的小时增量。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HourlyDelta {
    pub app: AppMeta,
    pub date: String,
    pub hour: u8,
    pub seconds: u64,
}

pub fn migrations() -> Vec<ModuleMigration> {
    vec![ModuleMigration::new(
        MODULE_ID,
        1,
        "创建应用使用时长元数据、小时聚合与会话表",
        "CREATE TABLE IF NOT EXISTS tracked_apps (
                app_id TEXT PRIMARY KEY,
                app_name TEXT NOT NULL,
                executable_path TEXT,
                category TEXT NOT NULL DEFAULT 'other',
                ignored INTEGER NOT NULL DEFAULT 0,
                window_title TEXT,
                first_seen_at INTEGER NOT NULL,
                last_seen_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS app_usage_hourly (
                date_str TEXT NOT NULL,
                hour_of_day INTEGER NOT NULL CHECK (hour_of_day BETWEEN 0 AND 23),
                app_id TEXT NOT NULL,
                active_seconds INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (date_str, hour_of_day, app_id)
            );
            CREATE INDEX IF NOT EXISTS idx_app_usage_hourly_date
                ON app_usage_hourly(date_str);
            CREATE INDEX IF NOT EXISTS idx_app_usage_hourly_app
                ON app_usage_hourly(app_id);
            CREATE TABLE IF NOT EXISTS app_usage_sessions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                app_id TEXT NOT NULL,
                app_name TEXT NOT NULL,
                window_title TEXT,
                started_at INTEGER NOT NULL,
                ended_at INTEGER NOT NULL,
                active_seconds INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_app_usage_sessions_started
                ON app_usage_sessions(started_at DESC);
            CREATE INDEX IF NOT EXISTS idx_app_usage_sessions_app
                ON app_usage_sessions(app_id, started_at DESC);",
    )]
}

fn upsert_app(tx: &Transaction<'_>, app: &AppMeta) -> CoreResult<()> {
    let now = Local::now().timestamp_millis();
    tx.execute(
        "INSERT INTO tracked_apps (
            app_id, app_name, executable_path, category, ignored, window_title,
            first_seen_at, last_seen_at
         ) VALUES (?1, ?2, ?3, 'other', 0, ?4, ?5, ?5)
         ON CONFLICT(app_id) DO UPDATE SET
            app_name = excluded.app_name,
            executable_path = COALESCE(excluded.executable_path, tracked_apps.executable_path),
            window_title = excluded.window_title,
            last_seen_at = excluded.last_seen_at",
        params![
            app.app_id,
            app.app_name,
            app.executable_path,
            app.window_title,
            now
        ],
    )?;
    Ok(())
}

/// 批量写入小时增量；空输入不发起事务。
pub(crate) fn write_deltas(ctx: &ModuleContext, deltas: &[HourlyDelta]) -> CoreResult<()> {
    if deltas.is_empty() {
        return Ok(());
    }
    ctx.db.with_conn(|conn| {
        let tx = conn.transaction()?;
        for delta in deltas {
            if delta.seconds == 0 {
                continue;
            }
            upsert_app(&tx, &delta.app)?;
            tx.execute(
                "INSERT INTO app_usage_hourly (date_str, hour_of_day, app_id, active_seconds)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(date_str, hour_of_day, app_id) DO UPDATE SET
                    active_seconds = app_usage_hourly.active_seconds + excluded.active_seconds",
                params![
                    delta.date,
                    i64::from(delta.hour),
                    delta.app.app_id,
                    delta.seconds as i64
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    })
}

/// 写入已结束会话；不足一秒的短暂切换不落盘。
pub(crate) fn insert_session(ctx: &ModuleContext, session: &ClosedSession) -> CoreResult<()> {
    if session.active_seconds == 0 || session.ended_at <= session.started_at {
        return Ok(());
    }
    ctx.db.with_conn(|conn| {
        let tx = conn.transaction()?;
        upsert_app(&tx, &session.app)?;
        tx.execute(
            "INSERT INTO app_usage_sessions (
                app_id, app_name, window_title, started_at, ended_at, active_seconds
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                session.app.app_id,
                session.app.app_name,
                session.app.window_title,
                session.started_at,
                session.ended_at,
                session.active_seconds as i64
            ],
        )?;
        tx.commit()?;
        Ok(())
    })
}

/// 读取忽略名单；采样线程用它即时跳过不计时的应用。
pub(crate) fn load_ignored(ctx: &ModuleContext) -> CoreResult<Vec<String>> {
    ctx.db.with_conn(|conn| {
        let mut stmt = conn.prepare("SELECT app_id FROM tracked_apps WHERE ignored = 1")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    })
}

pub fn set_app_category(
    ctx: &ModuleContext,
    app_id: &str,
    category: &str,
    ignored: bool,
) -> CoreResult<TrackedApp> {
    let app_id = app_id.trim();
    if app_id.is_empty() {
        return Err(CoreError::Config("Missing app_id".to_string()));
    }
    let category = crate::config::normalize_category(category)
        .ok_or_else(|| CoreError::Config(format!("Unknown category '{category}'")))?;
    let affected = ctx.db.with_conn(|conn| {
        Ok(conn.execute(
            "UPDATE tracked_apps SET category = ?1, ignored = ?2 WHERE app_id = ?3",
            params![category, i64::from(ignored), app_id],
        )?)
    })?;
    if affected == 0 {
        return Err(CoreError::Config(format!(
            "Application '{app_id}' has not been tracked yet"
        )));
    }
    ctx.db.with_conn(|conn| {
        conn.query_row(
            "SELECT app_id, app_name, executable_path, category, ignored, first_seen_at, last_seen_at
             FROM tracked_apps WHERE app_id = ?1",
            params![app_id],
            tracked_app_from_row,
        )
        .optional()
        .map_err(CoreError::from)?
        .ok_or_else(|| CoreError::Config(format!("Application '{app_id}' disappeared")))
    })
}

fn tracked_app_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TrackedApp> {
    Ok(TrackedApp {
        app_id: row.get(0)?,
        app_name: row.get(1)?,
        executable_path: row.get(2)?,
        category: row.get(3)?,
        ignored: row.get::<_, i64>(4)? != 0,
        first_seen_at: row.get(5)?,
        last_seen_at: row.get(6)?,
        total_seconds: 0,
    })
}

pub fn list_tracked_apps(ctx: &ModuleContext) -> CoreResult<Vec<TrackedApp>> {
    ctx.db.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT ta.app_id, ta.app_name, ta.executable_path, ta.category, ta.ignored,
                    ta.first_seen_at, ta.last_seen_at,
                    COALESCE((SELECT SUM(uh.active_seconds) FROM app_usage_hourly uh
                              WHERE uh.app_id = ta.app_id), 0)
             FROM tracked_apps ta
             ORDER BY ta.last_seen_at DESC, ta.app_id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            let mut app = tracked_app_from_row(row)?;
            app.total_seconds = row.get::<_, i64>(7)?.max(0) as u64;
            Ok(app)
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    })
}

fn parse_date(raw: &str) -> CoreResult<NaiveDate> {
    NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d")
        .map_err(|_| CoreError::Config(format!("Invalid date '{raw}', expected YYYY-MM-DD")))
}

fn enumerate_dates(start: NaiveDate, end: NaiveDate) -> CoreResult<Vec<String>> {
    if end < start {
        return Err(CoreError::Config(
            "End date must not be earlier than start date".to_string(),
        ));
    }
    let mut dates = Vec::new();
    let mut cursor = start;
    while cursor <= end {
        dates.push(cursor.format("%Y-%m-%d").to_string());
        if dates.len() > MAX_REPORT_DAYS {
            return Err(CoreError::Config(format!(
                "Date range is limited to {MAX_REPORT_DAYS} days"
            )));
        }
        cursor = cursor.succ_opt().ok_or_else(|| {
            CoreError::Config("Date range exceeds the supported calendar".to_string())
        })?;
    }
    Ok(dates)
}

pub fn overview(
    ctx: &ModuleContext,
    start_date: &str,
    end_date: &str,
) -> CoreResult<UsageOverview> {
    let start = parse_date(start_date)?;
    let end = parse_date(end_date)?;
    let dates = enumerate_dates(start, end)?;
    let start_str = dates.first().cloned().unwrap_or_default();
    let end_str = dates.last().cloned().unwrap_or_default();

    let mut day_totals: BTreeMap<String, u64> =
        dates.iter().map(|date| (date.clone(), 0)).collect();
    let mut app_totals: BTreeMap<String, AppUsageSummary> = BTreeMap::new();

    ctx.db.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT uh.date_str, uh.app_id, ta.app_name, ta.category, ta.ignored,
                    SUM(uh.active_seconds)
             FROM app_usage_hourly uh
             LEFT JOIN tracked_apps ta ON ta.app_id = uh.app_id
             WHERE uh.date_str >= ?1 AND uh.date_str <= ?2
               AND COALESCE(ta.ignored, 0) = 0
             GROUP BY uh.date_str, uh.app_id, ta.app_name, ta.category, ta.ignored
             ORDER BY uh.date_str ASC, SUM(uh.active_seconds) DESC",
        )?;
        let rows = stmt.query_map(params![start_str, end_str], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, i64>(5)?.max(0) as u64,
            ))
        })?;
        for row in rows {
            let (date, app_id, app_name, category, ignored, seconds) = row?;
            if seconds == 0 {
                continue;
            }
            *day_totals.entry(date).or_insert(0) += seconds;
            let entry = app_totals
                .entry(app_id.clone())
                .or_insert_with(|| AppUsageSummary {
                    app_id: app_id.clone(),
                    app_name: app_name.unwrap_or_else(|| app_id.clone()),
                    category: category.unwrap_or_else(|| "other".to_string()),
                    ignored: ignored.unwrap_or(0) != 0,
                    active_seconds: 0,
                });
            entry.active_seconds += seconds;
        }
        Ok(())
    })?;

    let total_seconds = day_totals.values().sum();
    let days: Vec<DailyUsage> = dates
        .into_iter()
        .map(|date| DailyUsage {
            active_seconds: day_totals.get(&date).copied().unwrap_or(0),
            date,
        })
        .collect();
    let mut apps: Vec<AppUsageSummary> = app_totals.into_values().collect();
    apps.sort_by(|a, b| {
        b.active_seconds
            .cmp(&a.active_seconds)
            .then_with(|| a.app_name.cmp(&b.app_name))
    });

    // 分类占比基于全部应用计算，避免被前 200 名截断。
    let mut category_totals: BTreeMap<String, u64> = BTreeMap::new();
    for app in &apps {
        *category_totals.entry(app.category.clone()).or_insert(0) += app.active_seconds;
    }
    let mut categories: Vec<CategoryUsage> = category_totals
        .into_iter()
        .map(|(category, active_seconds)| CategoryUsage {
            category,
            active_seconds,
        })
        .collect();
    categories.sort_by(|a, b| {
        b.active_seconds
            .cmp(&a.active_seconds)
            .then_with(|| a.category.cmp(&b.category))
    });
    apps.truncate(200);

    Ok(UsageOverview {
        start_date: start_str,
        end_date: end_str,
        total_seconds,
        days,
        apps,
        categories,
    })
}

pub fn hourly(ctx: &ModuleContext, date: &str) -> CoreResult<HourlyBreakdown> {
    let date = parse_date(date)?.format("%Y-%m-%d").to_string();
    let mut hours: Vec<HourlyUsage> = (0..24)
        .map(|hour| HourlyUsage {
            hour: hour as u8,
            active_seconds: 0,
            apps: Vec::new(),
        })
        .collect();

    ctx.db.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT uh.hour_of_day, uh.app_id, ta.app_name, ta.category, ta.ignored,
                    SUM(uh.active_seconds)
             FROM app_usage_hourly uh
             LEFT JOIN tracked_apps ta ON ta.app_id = uh.app_id
             WHERE uh.date_str = ?1 AND COALESCE(ta.ignored, 0) = 0
             GROUP BY uh.hour_of_day, uh.app_id, ta.app_name, ta.category, ta.ignored
             ORDER BY uh.hour_of_day ASC, SUM(uh.active_seconds) DESC",
        )?;
        let rows = stmt.query_map(params![date], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, i64>(5)?.max(0) as u64,
            ))
        })?;
        for row in rows {
            let (hour, app_id, app_name, category, ignored, seconds) = row?;
            if !(0..24).contains(&hour) || seconds == 0 {
                continue;
            }
            let bucket = &mut hours[hour as usize];
            bucket.active_seconds += seconds;
            bucket.apps.push(AppUsageSummary {
                app_id: app_id.clone(),
                app_name: app_name.unwrap_or_else(|| app_id.clone()),
                category: category.unwrap_or_else(|| "other".to_string()),
                ignored: ignored.unwrap_or(0) != 0,
                active_seconds: seconds,
            });
        }
        Ok(())
    })?;

    let total_seconds = hours.iter().map(|hour| hour.active_seconds).sum();
    Ok(HourlyBreakdown {
        date,
        total_seconds,
        hours,
    })
}

/// 本地“今天”，用于前端默认日期范围。
pub fn today_local() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_ranges_are_validated_and_filled() {
        let dates = enumerate_dates(
            NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
            NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(),
        )
        .unwrap();
        assert_eq!(dates, vec!["2026-10-31", "2026-11-01", "2026-11-02"]);
        assert!(enumerate_dates(
            NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(),
            NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
        )
        .is_err());
        assert!(parse_date("2026-13-01").is_err());
    }
}
