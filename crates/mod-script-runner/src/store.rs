//! SQLite 持久化：脚本 CRUD 与执行记录，表结构由模块迁移创建。
use crate::model::{RunStatus, ScriptRecord, ScriptRunRecord, ScriptType, ScriptUpsert};
use app_core::{CoreError, CoreResult, ModuleContext, ModuleMigration};
use rusqlite::{params, OptionalExtension};

pub fn migrations() -> Vec<ModuleMigration> {
    vec![ModuleMigration::new(
        crate::config::MODULE_ID,
        1,
        "创建 custom_scripts 脚本库与 script_run_logs 执行记录表",
        "CREATE TABLE IF NOT EXISTS custom_scripts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                script_type TEXT NOT NULL,
                is_inline INTEGER NOT NULL DEFAULT 1,
                content_or_path TEXT NOT NULL,
                args_json TEXT NOT NULL DEFAULT '[]',
                working_dir TEXT,
                hotkey TEXT,
                show_in_tray INTEGER NOT NULL DEFAULT 0,
                timeout_secs INTEGER NOT NULL DEFAULT 60,
                output_handling TEXT NOT NULL DEFAULT 'stream',
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_custom_scripts_updated
                ON custom_scripts(updated_at DESC, id DESC);
            CREATE TABLE IF NOT EXISTS script_run_logs (
                run_id TEXT PRIMARY KEY,
                script_id INTEGER NOT NULL,
                script_name TEXT NOT NULL,
                status TEXT NOT NULL,
                exit_code INTEGER,
                timed_out INTEGER NOT NULL DEFAULT 0,
                pid INTEGER,
                started_at INTEGER NOT NULL,
                finished_at INTEGER,
                duration_ms INTEGER,
                stdout_tail TEXT NOT NULL DEFAULT '',
                stderr_tail TEXT NOT NULL DEFAULT '',
                error_message TEXT,
                log_bytes INTEGER NOT NULL DEFAULT 0,
                dropped_bytes INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_script_run_logs_script
                ON script_run_logs(script_id, started_at DESC);
            CREATE INDEX IF NOT EXISTS idx_script_run_logs_started
                ON script_run_logs(started_at DESC);",
    )]
}

fn script_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScriptRecord> {
    let type_str: String = row.get(2)?;
    let args_json: String = row.get(5)?;
    Ok(ScriptRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        script_type: ScriptType::from_db_str(&type_str).unwrap_or(ScriptType::Shell),
        is_inline: row.get::<_, i64>(3)? != 0,
        content_or_path: row.get(4)?,
        args: serde_json::from_str(&args_json).unwrap_or_default(),
        working_dir: row.get(6)?,
        hotkey: row.get(7)?,
        show_in_tray: row.get::<_, i64>(8)? != 0,
        timeout_secs: row.get::<_, i64>(9)?.max(1) as u64,
        output_handling: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
        hotkey_active: false,
    })
}

const SCRIPT_COLUMNS: &str = "id, name, script_type, is_inline, content_or_path, args_json,
     working_dir, hotkey, show_in_tray, timeout_secs, output_handling, created_at, updated_at";

pub fn list_scripts(ctx: &ModuleContext) -> CoreResult<Vec<ScriptRecord>> {
    ctx.db.with_conn(|conn| {
        let sql = format!(
            "SELECT {SCRIPT_COLUMNS} FROM custom_scripts ORDER BY updated_at DESC, id DESC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], script_from_row)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        Ok(records)
    })
}

pub fn get_script(ctx: &ModuleContext, id: i64) -> CoreResult<Option<ScriptRecord>> {
    ctx.db.with_conn(|conn| {
        let sql = format!("SELECT {SCRIPT_COLUMNS} FROM custom_scripts WHERE id = ?1");
        conn.query_row(&sql, params![id], script_from_row)
            .optional()
            .map_err(CoreError::from)
    })
}

pub fn insert_script(
    ctx: &ModuleContext,
    input: &ScriptUpsert,
    timeout_secs: u64,
) -> CoreResult<ScriptRecord> {
    let now = chrono::Utc::now().timestamp_millis();
    let args_json = serde_json::to_string(&input.args)?;
    let id = ctx.db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO custom_scripts (
                name, script_type, is_inline, content_or_path, args_json, working_dir,
                hotkey, show_in_tray, timeout_secs, output_handling, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
            params![
                input.name.trim(),
                input.script_type.as_str(),
                i64::from(input.is_inline),
                input.content_or_path,
                args_json,
                input.working_dir,
                input.hotkey,
                i64::from(input.show_in_tray),
                timeout_secs as i64,
                input.output_handling,
                now,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    })?;
    get_script(ctx, id)?
        .ok_or_else(|| CoreError::Internal(anyhow::anyhow!("script insert disappeared")))
}

pub fn update_script(
    ctx: &ModuleContext,
    id: i64,
    input: &ScriptUpsert,
    timeout_secs: u64,
) -> CoreResult<ScriptRecord> {
    let now = chrono::Utc::now().timestamp_millis();
    let args_json = serde_json::to_string(&input.args)?;
    let affected = ctx.db.with_conn(|conn| {
        Ok(conn.execute(
            "UPDATE custom_scripts SET
                name = ?1, script_type = ?2, is_inline = ?3, content_or_path = ?4,
                args_json = ?5, working_dir = ?6, hotkey = ?7, show_in_tray = ?8,
                timeout_secs = ?9, output_handling = ?10, updated_at = ?11
             WHERE id = ?12",
            params![
                input.name.trim(),
                input.script_type.as_str(),
                i64::from(input.is_inline),
                input.content_or_path,
                args_json,
                input.working_dir,
                input.hotkey,
                i64::from(input.show_in_tray),
                timeout_secs as i64,
                input.output_handling,
                now,
                id,
            ],
        )?)
    })?;
    if affected == 0 {
        return Err(CoreError::Config(format!("script {id} not found")));
    }
    get_script(ctx, id)?
        .ok_or_else(|| CoreError::Internal(anyhow::anyhow!("script {id} disappeared")))
}

pub fn delete_script(ctx: &ModuleContext, id: i64) -> CoreResult<bool> {
    ctx.db.with_conn(|conn| {
        let affected = conn.execute("DELETE FROM custom_scripts WHERE id = ?1", params![id])?;
        Ok(affected > 0)
    })
}

pub fn mark_stale_runs_interrupted(ctx: &ModuleContext) -> CoreResult<usize> {
    let now = chrono::Utc::now().timestamp_millis();
    ctx.db.with_conn(|conn| {
        let affected = conn.execute(
            "UPDATE script_run_logs
             SET status = 'interrupted', finished_at = ?1,
                 error_message = COALESCE(error_message, 'application stopped while script was running')
             WHERE status = 'running'",
            params![now],
        )?;
        Ok(affected)
    })
}

pub fn insert_running_run(
    ctx: &ModuleContext,
    run_id: &str,
    script: &ScriptRecord,
    pid: Option<u32>,
) -> CoreResult<()> {
    let now = chrono::Utc::now().timestamp_millis();
    ctx.db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO script_run_logs (
                run_id, script_id, script_name, status, exit_code, timed_out, pid,
                started_at, finished_at, duration_ms, stdout_tail, stderr_tail,
                error_message, log_bytes, dropped_bytes
            ) VALUES (?1, ?2, ?3, 'running', NULL, 0, ?4, ?5, NULL, NULL, '', '', NULL, 0, 0)",
            params![run_id, script.id, script.name, pid, now],
        )?;
        Ok(())
    })
}

pub fn mark_spawn_failed(
    ctx: &ModuleContext,
    run_id: &str,
    script: &ScriptRecord,
    error: &str,
) -> CoreResult<()> {
    let now = chrono::Utc::now().timestamp_millis();
    ctx.db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO script_run_logs (
                run_id, script_id, script_name, status, exit_code, timed_out, pid,
                started_at, finished_at, duration_ms, stdout_tail, stderr_tail,
                error_message, log_bytes, dropped_bytes
            ) VALUES (?1, ?2, ?3, 'spawn_failed', NULL, 0, NULL, ?4, ?4, 0, '', '', ?5, 0, 0)",
            params![run_id, script.id, script.name, now, error],
        )?;
        Ok(())
    })
}

#[allow(clippy::too_many_arguments)]
pub fn finish_run(
    ctx: &ModuleContext,
    record: &ScriptRunRecord,
    keep_run_logs: usize,
) -> CoreResult<()> {
    ctx.db.with_conn(|conn| {
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE script_run_logs SET
                status = ?1, exit_code = ?2, timed_out = ?3, pid = ?4,
                started_at = ?5, finished_at = ?6, duration_ms = ?7,
                stdout_tail = ?8, stderr_tail = ?9, error_message = ?10,
                log_bytes = ?11, dropped_bytes = ?12
             WHERE run_id = ?13",
            params![
                record.status.as_str(),
                record.exit_code,
                i64::from(record.timed_out),
                record.pid,
                record.started_at,
                record.finished_at,
                record.duration_ms.map(|v| v as i64),
                record.stdout_tail,
                record.stderr_tail,
                record.error_message,
                record.log_bytes as i64,
                record.dropped_bytes as i64,
                record.run_id,
            ],
        )?;
        tx.execute(
            "DELETE FROM script_run_logs
             WHERE status != 'running' AND run_id NOT IN (
                SELECT run_id FROM script_run_logs
                ORDER BY started_at DESC, run_id DESC LIMIT ?1
             )",
            params![keep_run_logs.max(1) as i64],
        )?;
        tx.commit()?;
        Ok(())
    })
}

fn run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScriptRunRecord> {
    let status: String = row.get(3)?;
    Ok(ScriptRunRecord {
        run_id: row.get(0)?,
        script_id: row.get(1)?,
        script_name: row.get(2)?,
        status: RunStatus::from_db_str(&status),
        exit_code: row.get(4)?,
        timed_out: row.get::<_, i64>(5)? != 0,
        pid: row.get::<_, Option<i64>>(6)?.map(|v| v as u32),
        started_at: row.get(7)?,
        finished_at: row.get(8)?,
        duration_ms: row.get::<_, Option<i64>>(9)?.map(|v| v as u64),
        stdout_tail: row.get(10)?,
        stderr_tail: row.get(11)?,
        error_message: row.get(12)?,
        log_bytes: row.get::<_, i64>(13)?.max(0) as usize,
        dropped_bytes: row.get::<_, i64>(14)?.max(0) as usize,
    })
}

const RUN_COLUMNS: &str = "run_id, script_id, script_name, status, exit_code, timed_out, pid,
     started_at, finished_at, duration_ms, stdout_tail, stderr_tail, error_message,
     log_bytes, dropped_bytes";

pub fn get_run(ctx: &ModuleContext, run_id: &str) -> CoreResult<Option<ScriptRunRecord>> {
    ctx.db.with_conn(|conn| {
        let sql = format!("SELECT {RUN_COLUMNS} FROM script_run_logs WHERE run_id = ?1");
        conn.query_row(&sql, params![run_id], run_from_row)
            .optional()
            .map_err(CoreError::from)
    })
}

pub fn list_runs(
    ctx: &ModuleContext,
    script_id: Option<i64>,
    limit: usize,
) -> CoreResult<Vec<ScriptRunRecord>> {
    let limit = limit.clamp(1, 200) as i64;
    ctx.db.with_conn(|conn| {
        let mut records = Vec::new();
        if let Some(script_id) = script_id {
            let sql = format!(
                "SELECT {RUN_COLUMNS} FROM script_run_logs
                 WHERE script_id = ?1 ORDER BY started_at DESC, run_id DESC LIMIT ?2"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![script_id, limit], run_from_row)?;
            for row in rows {
                records.push(row?);
            }
        } else {
            let sql = format!(
                "SELECT {RUN_COLUMNS} FROM script_run_logs
                 ORDER BY started_at DESC, run_id DESC LIMIT ?1"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![limit], run_from_row)?;
            for row in rows {
                records.push(row?);
            }
        }
        Ok(records)
    })
}
