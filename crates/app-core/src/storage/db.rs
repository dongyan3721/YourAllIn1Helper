//! Local SQLite storage engine with WAL mode, automatic pre-upgrade backup,
//! and atomic incremental schema migration across core and feature modules.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use chrono::Utc;
use parking_lot::{Mutex, MutexGuard};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{info, warn};
use crate::error::{CoreError, CoreResult};

/// Incremental SQL migration step declared by either `app-core` or an `AppModule`.
#[derive(Debug, Clone)]
pub struct ModuleMigration {
    /// Monotonically increasing migration version within `module_id` (starting at 1).
    pub version: u32,
    /// Owning module ID (e.g., `"core"`, `"clipboard"`, `"sticky_note"`, `"app_usage"`).
    pub module_id: &'static str,
    /// Short human-readable summary of the schema change.
    pub description: &'static str,
    /// Idempotent or incremental SQL statements executed inside a transaction.
    pub sql: &'static str,
}

impl ModuleMigration {
    pub const fn new(
        module_id: &'static str,
        version: u32,
        description: &'static str,
        sql: &'static str,
    ) -> Self {
        Self {
            version,
            module_id,
            description,
            sql,
        }
    }

    /// Computes a deterministic SHA-256 hex checksum of the migration SQL.
    pub fn checksum_hex(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.sql.as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

/// Record of an applied migration persisted in `_schema_migrations`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedMigrationRecord {
    pub module_id: String,
    pub version: u32,
    pub description: String,
    pub checksum: String,
    pub applied_at: i64,
}

/// Thread-safe SQLite connection pool/handle optimized for single-user desktop workloads.
#[derive(Clone)]
pub struct DbPool {
    conn: Arc<Mutex<Connection>>,
    db_path: Option<PathBuf>,
}

impl DbPool {
    /// Opens or creates the SQLite database at `db_path`, enables WAL & foreign keys,
    /// and initializes the `_schema_migrations` tracking table.
    pub fn open(db_path: impl AsRef<Path>) -> CoreResult<Self> {
        let path = db_path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }

        let conn = Connection::open(&path)?;
        Self::configure_pragmas(&conn)?;
        Self::init_migration_table(&conn)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            db_path: Some(path),
        })
    }

    /// Opens an in-memory SQLite database for fast unit and integration testing.
    pub fn open_in_memory() -> CoreResult<Self> {
        let conn = Connection::open_in_memory()?;
        Self::configure_pragmas(&conn)?;
        Self::init_migration_table(&conn)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            db_path: None,
        })
    }

    /// Acquires an exclusive guard to the underlying `rusqlite::Connection`.
    pub fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock()
    }

    /// Executes a closure with exclusive access to the SQLite connection.
    pub fn with_conn<F, T>(&self, f: F) -> CoreResult<T>
    where
        F: FnOnce(&mut Connection) -> CoreResult<T>,
    {
        let mut guard = self.conn.lock();
        f(&mut guard)
    }

    /// Returns the filesystem path of `app.db`, if backed by disk.
    pub fn db_path(&self) -> Option<&Path> {
        self.db_path.as_deref()
    }

    fn configure_pragmas(conn: &Connection) -> CoreResult<()> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA temp_store = MEMORY;
             PRAGMA busy_timeout = 5000;",
        )?;
        Ok(())
    }

    fn init_migration_table(conn: &Connection) -> CoreResult<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS _schema_migrations (
                module_id   TEXT NOT NULL,
                version     INTEGER NOT NULL,
                description TEXT NOT NULL,
                checksum    TEXT NOT NULL,
                applied_at  INTEGER NOT NULL,
                PRIMARY KEY (module_id, version)
            );
            CREATE TABLE IF NOT EXISTS _app_kv_meta (
                key         TEXT PRIMARY KEY,
                value_json  TEXT NOT NULL,
                updated_at  INTEGER NOT NULL
            );",
        )?;
        Ok(())
    }

    /// Applies all pending migrations across `core` and registered modules.
    ///
    /// If any migration is pending on a disk-backed database that already contains user data,
    /// automatically creates a timestamped backup in `<db_dir>/backups/app.db.pre_migrate.<ts>.bak`
    /// before executing the migrations inside atomic transactions.
    pub fn run_migrations(&self, migrations: &[ModuleMigration]) -> CoreResult< usize > {
        let mut sorted: Vec<ModuleMigration> = migrations.to_vec();
        // Ensure deterministic order: `core` first, then alphabetical by `module_id`, then ascending `version`.
        sorted.sort_by(|a, b| {
            let a_is_core = a.module_id == "core";
            let b_is_core = b.module_id == "core";
            b_is_core
                .cmp(&a_is_core)
                .then_with(|| a.module_id.cmp(b.module_id))
                .then_with(|| a.version.cmp(&b.version))
        });

        let applied = self.list_applied_migrations()?;
        let pending: Vec<ModuleMigration> = sorted
            .into_iter()
            .filter(|m| {
                !applied
                    .iter()
                    .any(|a| a.module_id == m.module_id && a.version == m.version)
            })
            .collect();

        if pending.is_empty() {
            return Ok(0);
        }

        // If the database already had migrations applied previously and is disk-backed, back it up first!
        if !applied.is_empty() {
            if let Err(err) = self.create_backup_snapshot("pre_migrate") {
                warn!("Pre-migration backup warning (continuing): {}", err);
            }
        }

        let mut conn = self.conn.lock();
        let mut applied_count = 0;

        for migration in pending {
            let tx = conn.transaction()?;
            tx.execute_batch(migration.sql).map_err(|e| CoreError::Migration {
                version: migration.version,
                module_id: migration.module_id.to_string(),
                reason: e.to_string(),
            })?;

            let now_ts = Utc::now().timestamp();
            let checksum = migration.checksum_hex();
            tx.execute(
                "INSERT INTO _schema_migrations (module_id, version, description, checksum, applied_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    migration.module_id,
                    migration.version,
                    migration.description,
                    checksum,
                    now_ts
                ],
            )?;

            tx.commit()?;
            applied_count += 1;
            info!(
                module_id = migration.module_id,
                version = migration.version,
                description = migration.description,
                "Applied SQLite schema migration"
            );
        }

        Ok(applied_count)
    }

    /// Creates a consistent backup of the SQLite database in the `backups/` subdirectory.
    pub fn create_backup_snapshot(&self, tag: &str) -> CoreResult<Option<PathBuf>> {
        let Some(db_path) = &self.db_path else {
            return Ok(None);
        };
        if !db_path.exists() {
            return Ok(None);
        }

        let parent = db_path.parent().unwrap_or_else(|| Path::new("."));
        let backup_dir = parent.join("backups");
        fs::create_dir_all(&backup_dir)?;

        let ts = Utc::now().format("%Y%m%d_%H%M%S");
        let backup_file = backup_dir.join(format!("app.db.{}.{}.bak", tag, ts));

        let conn = self.conn.lock();
        let mut dst_conn = Connection::open(&backup_file)?;
        {
            let backup = rusqlite::backup::Backup::new(&conn, &mut dst_conn)?;
            backup.run_to_completion(100, std::time::Duration::from_millis(10), None)?;
        }

        Ok(Some(backup_file))
    }

    /// Lists all applied migrations ordered by `applied_at` and `(module_id, version)`.
    pub fn list_applied_migrations(&self) -> CoreResult<Vec<AppliedMigrationRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT module_id, version, description, checksum, applied_at
             FROM _schema_migrations
             ORDER BY module_id ASC, version ASC",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(AppliedMigrationRecord {
                module_id: row.get(0)?,
                version: row.get(1)?,
                description: row.get(2)?,
                checksum: row.get(3)?,
                applied_at: row.get(4)?,
            })
        })?;

        let mut records = Vec::new();
        for r in rows {
            records.push(r?);
        }
        Ok(records)
    }
}
