//! `mod-clipboard` — Module 02: Ditto-style Clipboard Manager.
//!
//! Features:
//! - **SQLite `clipboard_items` Table**: Stores text and image clipboard records with unique `content_hash`.
//! - **SHA-256 Deduplication**: Re-copying an existing item updates its `updated_at` timestamp to float
//!   it to the top of the list without creating duplicate rows.
//! - **LRU-100 Automatic Eviction**: Enforces `config.clipboard.max_history_items` (default 100) for
//!   unpinned items while permanently preserving pinned (`is_pinned = 1`) items.
//! - **Anti-Pollution Guard**: Checks `EventBus::is_clipboard_suppressed()` before recording changes so
//!   simulated `Ctrl+C` (Translator) and secret password copies (Password Vault) are ignored.
//! - **Quick Paste**: Restores item content and invokes `os_hal::InputSimulator::send_paste_shortcut()`.

use app_core::{
    AppModule, CoreError, CoreResult, ModuleContext, ModuleMigration, ShortcutSpec, SystemEvent,
    WindowSpec,
};
use chrono::Utc;
use os_hal::{InputSimulator, MockOsHal};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tracing::info;

/// Type of clipboard entry stored in `clipboard_items`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardItemType {
    Text,
    Image,
}

impl ClipboardItemType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
        }
    }

    pub fn from_db_str(s: &str) -> Self {
        match s {
            "image" => Self::Image,
            _ => Self::Text,
        }
    }
}

/// A single clipboard history record returned to the UI (`clipboard-popup`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardRecord {
    pub id: i64,
    pub item_type: ClipboardItemType,
    pub content_text: Option<String>,
    pub blob_rel_path: Option<String>,
    pub thumb_base64: Option<String>,
    pub content_hash: String,
    pub source_app_name: Option<String>,
    pub is_pinned: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Module 02 implementation (`mod-clipboard`).
pub struct ClipboardModule {
    input_simulator: Arc<dyn InputSimulator>,
}

impl Default for ClipboardModule {
    fn default() -> Self {
        Self::new(Arc::new(MockOsHal::new()))
    }
}

impl ClipboardModule {
    pub fn new(input_simulator: Arc<dyn InputSimulator>) -> Self {
        Self { input_simulator }
    }

    /// Computes a deterministic SHA-256 hex digest for deduplication.
    pub fn compute_hash(prefix: &str, bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(prefix.as_bytes());
        hasher.update(b":");
        hasher.update(bytes);
        format!("{:x}", hasher.finalize())
    }

    /// Captures a text entry into `clipboard_items` with deduplication and LRU eviction.
    /// Returns `Ok(None)` if clipboard capture is currently suppressed or disabled in config.
    pub fn capture_text(
        &self,
        ctx: &ModuleContext,
        text: &str,
        source_app_name: Option<&str>,
    ) -> CoreResult<Option<ClipboardRecord>> {
        if ctx.event_bus.is_clipboard_suppressed() {
            return Ok(None);
        }

        let (enabled, max_items) = {
            let cfg = ctx.config.read();
            (cfg.clipboard.enabled, cfg.clipboard.max_history_items)
        };
        if !enabled || text.trim().is_empty() {
            return Ok(None);
        }

        let content_hash = Self::compute_hash("text", text.as_bytes());
        let now = Utc::now().timestamp_millis();

        let item_id = ctx.db.with_conn(|conn| {
            let tx = conn.transaction()?;
            let effective_now = Self::next_monotonic_timestamp(&tx, now)?;

            let existing_id: Option<i64> = tx
                .query_row(
                    "SELECT id FROM clipboard_items WHERE content_hash = ?1",
                    params![content_hash],
                    |row| row.get(0),
                )
                .optional()?;

            let record_id = if let Some(id) = existing_id {
                tx.execute(
                    "UPDATE clipboard_items
                     SET updated_at = ?1, source_app_name = COALESCE(?2, source_app_name)
                     WHERE id = ?3",
                    params![effective_now, source_app_name, id],
                )?;
                id
            } else {
                tx.execute(
                    "INSERT INTO clipboard_items (
                        item_type, content_text, blob_rel_path, thumb_base64,
                        content_hash, source_app_name, is_pinned, created_at, updated_at
                    ) VALUES ('text', ?1, NULL, NULL, ?2, ?3, 0, ?4, ?4)",
                    params![text, content_hash, source_app_name, effective_now],
                )?;
                tx.last_insert_rowid()
            };

            Self::evict_lru_unpinned(&tx, max_items)?;
            tx.commit()?;
            Ok(record_id)
        })?;

        ctx.event_bus.publish(SystemEvent::ClipboardUpdated {
            item_id,
            item_type: "text".to_string(),
        });

        self.get_item_by_id(ctx, item_id)
    }

    /// Captures an image entry into `clipboard_items` with deduplication and LRU eviction.
    pub fn capture_image(
        &self,
        ctx: &ModuleContext,
        image_bytes: &[u8],
        blob_rel_path: &str,
        thumb_base64: Option<&str>,
        source_app_name: Option<&str>,
    ) -> CoreResult<Option<ClipboardRecord>> {
        if ctx.event_bus.is_clipboard_suppressed() {
            return Ok(None);
        }

        let (enabled, capture_images, max_items) = {
            let cfg = ctx.config.read();
            (
                cfg.clipboard.enabled,
                cfg.clipboard.capture_images,
                cfg.clipboard.max_history_items,
            )
        };
        if !enabled || !capture_images || image_bytes.is_empty() {
            return Ok(None);
        }

        let content_hash = Self::compute_hash("image", image_bytes);
        let now = Utc::now().timestamp_millis();

        let item_id = ctx.db.with_conn(|conn| {
            let tx = conn.transaction()?;
            let effective_now = Self::next_monotonic_timestamp(&tx, now)?;

            let existing_id: Option<i64> = tx
                .query_row(
                    "SELECT id FROM clipboard_items WHERE content_hash = ?1",
                    params![content_hash],
                    |row| row.get(0),
                )
                .optional()?;

            let record_id = if let Some(id) = existing_id {
                tx.execute(
                    "UPDATE clipboard_items
                     SET updated_at = ?1, source_app_name = COALESCE(?2, source_app_name)
                     WHERE id = ?3",
                    params![effective_now, source_app_name, id],
                )?;
                id
            } else {
                tx.execute(
                    "INSERT INTO clipboard_items (
                        item_type, content_text, blob_rel_path, thumb_base64,
                        content_hash, source_app_name, is_pinned, created_at, updated_at
                    ) VALUES ('image', NULL, ?1, ?2, ?3, ?4, 0, ?5, ?5)",
                    params![
                        blob_rel_path,
                        thumb_base64,
                        content_hash,
                        source_app_name,
                        effective_now
                    ],
                )?;
                tx.last_insert_rowid()
            };

            Self::evict_lru_unpinned(&tx, max_items)?;
            tx.commit()?;
            Ok(record_id)
        })?;

        ctx.event_bus.publish(SystemEvent::ClipboardUpdated {
            item_id,
            item_type: "image".to_string(),
        });

        self.get_item_by_id(ctx, item_id)
    }

    /// Queries clipboard history ordered by `is_pinned DESC, updated_at DESC, id DESC`.
    pub fn list_items(
        &self,
        ctx: &ModuleContext,
        keyword: Option<&str>,
        limit: usize,
    ) -> CoreResult<Vec<ClipboardRecord>> {
        let effective_limit = limit.clamp(1, 500) as i64;
        let search_pattern = keyword
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| format!("%{}%", s));

        ctx.db.with_conn(|conn| {
            let mut records = Vec::new();
            if let Some(pattern) = search_pattern {
                let mut stmt = conn.prepare(
                    "SELECT id, item_type, content_text, blob_rel_path, thumb_base64,
                            content_hash, source_app_name, is_pinned, created_at, updated_at
                     FROM clipboard_items
                     WHERE content_text LIKE ?1 OR source_app_name LIKE ?1
                     ORDER BY is_pinned DESC, updated_at DESC, id DESC
                     LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![pattern, effective_limit], Self::map_row)?;
                for r in rows {
                    records.push(r?);
                }
            } else {
                let mut stmt = conn.prepare(
                    "SELECT id, item_type, content_text, blob_rel_path, thumb_base64,
                            content_hash, source_app_name, is_pinned, created_at, updated_at
                     FROM clipboard_items
                     ORDER BY is_pinned DESC, updated_at DESC, id DESC
                     LIMIT ?1",
                )?;
                let rows = stmt.query_map(params![effective_limit], Self::map_row)?;
                for r in rows {
                    records.push(r?);
                }
            }
            Ok(records)
        })
    }

    /// Fetches a single clipboard record by its primary key `id`.
    pub fn get_item_by_id(
        &self,
        ctx: &ModuleContext,
        id: i64,
    ) -> CoreResult<Option<ClipboardRecord>> {
        ctx.db.with_conn(|conn| {
            conn.query_row(
                "SELECT id, item_type, content_text, blob_rel_path, thumb_base64,
                        content_hash, source_app_name, is_pinned, created_at, updated_at
                 FROM clipboard_items
                 WHERE id = ?1",
                params![id],
                Self::map_row,
            )
            .optional()
            .map_err(CoreError::from)
        })
    }

    /// Toggles the `is_pinned` status of a clipboard item so it is exempt from LRU-100 eviction.
    pub fn toggle_pin(&self, ctx: &ModuleContext, id: i64) -> CoreResult<bool> {
        ctx.db.with_conn(|conn| {
            conn.execute(
                "UPDATE clipboard_items
                 SET is_pinned = CASE WHEN is_pinned = 1 THEN 0 ELSE 1 END
                 WHERE id = ?1",
                params![id],
            )?;
            let new_pin: i64 = conn.query_row(
                "SELECT is_pinned FROM clipboard_items WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )?;
            Ok(new_pin != 0)
        })
    }

    /// Deletes a single clipboard item by `id`.
    pub fn delete_item(&self, ctx: &ModuleContext, id: i64) -> CoreResult<bool> {
        ctx.db.with_conn(|conn| {
            let affected =
                conn.execute("DELETE FROM clipboard_items WHERE id = ?1", params![id])?;
            Ok(affected > 0)
        })
    }

    /// Clears all unpinned clipboard items while preserving pinned entries.
    pub fn clear_unpinned(&self, ctx: &ModuleContext) -> CoreResult<usize> {
        ctx.db.with_conn(|conn| {
            let deleted = conn.execute("DELETE FROM clipboard_items WHERE is_pinned = 0", [])?;
            Ok(deleted)
        })
    }

    /// Pastes the selected item to the foreground window, destroying the transient `clipboard-popup` first.
    pub fn paste_item(
        &self,
        ctx: &ModuleContext,
        id: i64,
        _as_plain_text: bool,
    ) -> CoreResult<ClipboardRecord> {
        let record = self.get_item_by_id(ctx, id)?.ok_or_else(|| {
            CoreError::Internal(anyhow::anyhow!("Clipboard item {} not found", id))
        })?;

        // Destroy the transient popup window before simulating Ctrl+V / Cmd+V
        let _ = ctx.window_pool.destroy_window("clipboard-popup");

        // Bump `updated_at` so recently pasted item rises to the top
        let now = Utc::now().timestamp_millis();
        ctx.db.with_conn(|conn| {
            let effective_now = Self::next_monotonic_timestamp(conn, now)?;
            conn.execute(
                "UPDATE clipboard_items SET updated_at = ?1 WHERE id = ?2",
                params![effective_now, id],
            )?;
            Ok(())
        })?;

        self.input_simulator
            .send_paste_shortcut()
            .map_err(|e| CoreError::Internal(anyhow::anyhow!(e)))?;

        Ok(record)
    }

    fn next_monotonic_timestamp(conn: &rusqlite::Connection, now: i64) -> CoreResult<i64> {
        let max_ts: i64 = conn.query_row(
            "SELECT COALESCE(MAX(updated_at), 0) FROM clipboard_items",
            [],
            |row| row.get(0),
        )?;
        Ok(now.max(max_ts + 1))
    }

    fn evict_lru_unpinned(conn: &rusqlite::Connection, max_unpinned: usize) -> CoreResult<()> {
        conn.execute(
            "DELETE FROM clipboard_items
             WHERE is_pinned = 0
               AND id NOT IN (
                   SELECT id FROM clipboard_items
                   WHERE is_pinned = 0
                   ORDER BY updated_at DESC, id DESC
                   LIMIT ?1
               )",
            params![max_unpinned as i64],
        )?;
        Ok(())
    }

    fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ClipboardRecord> {
        let type_str: String = row.get(1)?;
        let is_pinned_int: i64 = row.get(7)?;
        Ok(ClipboardRecord {
            id: row.get(0)?,
            item_type: ClipboardItemType::from_db_str(&type_str),
            content_text: row.get(2)?,
            blob_rel_path: row.get(3)?,
            thumb_base64: row.get(4)?,
            content_hash: row.get(5)?,
            source_app_name: row.get(6)?,
            is_pinned: is_pinned_int != 0,
            created_at: row.get(8)?,
            updated_at: row.get(9)?,
        })
    }
}

impl AppModule for ClipboardModule {
    fn id(&self) -> &'static str {
        "clipboard"
    }

    fn name(&self) -> &'static str {
        "剪切板管理器 (Ditto LRU-100)"
    }

    fn migrations(&self) -> Vec<ModuleMigration> {
        vec![ModuleMigration::new(
            "clipboard",
            1,
            "Create clipboard_items table with content_hash unique index and LRU index",
            "CREATE TABLE IF NOT EXISTS clipboard_items (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                item_type TEXT NOT NULL CHECK(item_type IN ('text', 'image')),
                content_text TEXT,
                blob_rel_path TEXT,
                thumb_base64 TEXT,
                content_hash TEXT NOT NULL UNIQUE,
                source_app_name TEXT,
                is_pinned INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_clipboard_lru
                ON clipboard_items(is_pinned DESC, updated_at DESC, id DESC);",
        )]
    }

    fn default_shortcuts(&self) -> Vec<ShortcutSpec> {
        vec![ShortcutSpec::new(
            "clipboard",
            "clipboard:toggle_popup",
            "PrimaryMod+Shift+V",
            "呼出/关闭剪切板历史弹窗",
        )]
    }

    fn on_init(&self, _ctx: &ModuleContext) -> CoreResult<()> {
        info!("ClipboardModule initialized (background listener ready, 0 WebViews)");
        Ok(())
    }

    fn on_shortcut(&self, action_id: &str, ctx: &ModuleContext) -> CoreResult<()> {
        if action_id == "clipboard:toggle_popup" {
            if ctx
                .window_pool
                .get_active_window("clipboard-popup")
                .is_some()
            {
                ctx.window_pool.destroy_window("clipboard-popup")?;
            } else {
                ctx.window_pool
                    .open_window(WindowSpec::clipboard_popup(480.0, 320.0))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_core::{AppKernelBuilder, TargetOs};

    #[test]
    fn test_clipboard_deduplication_lru_eviction_and_suppression() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mock_hal = MockOsHal::new();
        let clip_mod = ClipboardModule::new(Arc::new(mock_hal.clone()));

        let kernel = AppKernelBuilder::new(temp_dir.path())
            .with_in_memory_db(true)
            .with_target_os(TargetOs::MacOs)
            .register_module(ClipboardModule::new(Arc::new(mock_hal.clone())))
            .unwrap()
            .bootstrap()
            .unwrap();

        let ctx = kernel.context();

        // Lower max_history_items to 3 for testing LRU eviction
        kernel
            .update_config(|cfg| {
                cfg.clipboard.max_history_items = 3;
            })
            .unwrap();

        // 1. Insert 3 items and pin the first one
        let r1 = clip_mod
            .capture_text(ctx, "First pinned item", Some("Notes"))
            .unwrap()
            .unwrap();
        assert!(clip_mod.toggle_pin(ctx, r1.id).unwrap());

        let r2 = clip_mod
            .capture_text(ctx, "Second item", Some("Browser"))
            .unwrap()
            .unwrap();
        let _r3 = clip_mod
            .capture_text(ctx, "Third item", Some("Editor"))
            .unwrap()
            .unwrap();
        let _r4 = clip_mod
            .capture_text(ctx, "Fourth item", Some("Terminal"))
            .unwrap()
            .unwrap();

        // Re-copying "Second item" should deduplicate and move it to the top of unpinned items
        let r2_again = clip_mod
            .capture_text(ctx, "Second item", Some("Browser"))
            .unwrap()
            .unwrap();
        assert_eq!(r2.id, r2_again.id);

        // Adding a 5th item should evict the oldest unpinned item ("Third item"), keeping pinned r1 + 3 unpinned = 4 total
        let _r5 = clip_mod
            .capture_text(ctx, "Fifth item", Some("Terminal"))
            .unwrap()
            .unwrap();

        let all = clip_mod.list_items(ctx, None, 50).unwrap();
        assert_eq!(all.len(), 4);
        assert_eq!(all[0].content_text.as_deref(), Some("First pinned item"));
        assert!(all[0].is_pinned);

        // 2. Test suppression guard (e.g. during Password Vault copy or Translator Ctrl+C)
        ctx.event_bus
            .set_clipboard_suppression(true, "password_vault_secret");
        let suppressed = clip_mod
            .capture_text(ctx, "SuperSecretPassword123!", None)
            .unwrap();
        assert!(suppressed.is_none());
        ctx.event_bus.set_clipboard_suppression(false, "done");

        // 3. Test shortcut toggle & paste_item destroying transient popup
        kernel.trigger_action("clipboard:toggle_popup").unwrap();
        assert_eq!(ctx.window_pool.active_window_count(), 1);

        clip_mod.paste_item(ctx, r2.id, true).unwrap();
        assert_eq!(ctx.window_pool.active_window_count(), 0);
        assert_eq!(mock_hal.paste_trigger_count(), 1);
    }
}
