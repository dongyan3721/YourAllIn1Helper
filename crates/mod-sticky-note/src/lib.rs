//! `mod-sticky-note` — Module 04: Desktop Pinned Rich-Text Sticky Notes (Markdown + LaTeX).
//!
//! Features:
//! - **Level 2 Pinned Resident Window Strategy**: On `on_init`, queries `sticky_notes` for notes
//!   where `is_open = 1` and restores only those windows via `WindowPoolManager`. If no notes
//!   are open on desktop, 0 WebViews are spawned on startup.
//! - **Real-time Persistence**: Stores Markdown + LaTeX content (`content_md`), theme color,
//!   opacity, geometry (`pos_x`, `pos_y`, `width`, `height`), and pin state in `sticky_notes`.
//! - **Close & Destroy**: Closing a note marks `is_open = 0` in SQLite and immediately invokes
//!   `window_pool.destroy_window("sticky-note-{id}")` to free WebView memory.

use app_core::{
    AppModule, CoreError, CoreResult, ModuleContext, ModuleMigration, ShortcutSpec, WindowSpec,
};
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

/// Persistent data model for a desktop sticky note (`sticky_notes` table).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StickyNoteRecord {
    pub id: String,
    pub title: String,
    pub content_md: String,
    pub theme_color: String,
    pub opacity: f64,
    pub pos_x: Option<i32>,
    pub pos_y: Option<i32>,
    pub width: u32,
    pub height: u32,
    pub is_pinned: bool,
    pub is_open: bool,
    pub updated_at: i64,
}

/// Geometry and pin state update payload from the `sticky-note-{id}` window.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StickyNoteWindowStateUpdate {
    pub pos_x: Option<i32>,
    pub pos_y: Option<i32>,
    pub width: u32,
    pub height: u32,
    pub is_pinned: bool,
    pub opacity: f64,
}

/// Module 04 implementation (`mod-sticky-note`).
#[derive(Default, Clone)]
pub struct StickyNoteModule;

impl StickyNoteModule {
    pub fn new() -> Self {
        Self
    }

    /// Formats the canonical WebView window label for a sticky note ID.
    pub fn window_label_for(note_id: &str) -> String {
        format!("sticky-note-{}", note_id)
    }

    /// Creates a new sticky note in SQLite and immediately opens its `PinnedResident` WebView window.
    pub fn create_note(
        &self,
        ctx: &ModuleContext,
        title: Option<&str>,
        initial_markdown: Option<&str>,
        theme_color: Option<&str>,
    ) -> CoreResult<StickyNoteRecord> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().timestamp_millis();
        let note_title = title.unwrap_or("便笺").to_string();
        let content_md = initial_markdown.unwrap_or("").to_string();
        let color = theme_color.unwrap_or("amber").to_string();

        let record = StickyNoteRecord {
            id: id.clone(),
            title: note_title.clone(),
            content_md: content_md.clone(),
            theme_color: color.clone(),
            opacity: 0.95,
            pos_x: Some(240),
            pos_y: Some(180),
            width: 320,
            height: 300,
            is_pinned: true,
            is_open: true,
            updated_at: now,
        };

        ctx.db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sticky_notes (
                    id, title, content_md, theme_color, opacity,
                    pos_x, pos_y, width, height, is_pinned, is_open, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, 1, ?10)",
                params![
                    record.id,
                    record.title,
                    record.content_md,
                    record.theme_color,
                    record.opacity,
                    record.pos_x,
                    record.pos_y,
                    record.width,
                    record.height,
                    record.updated_at,
                ],
            )?;
            Ok(())
        })?;

        self.spawn_note_window(ctx, &record)?;
        Ok(record)
    }

    /// Retrieves a single sticky note by `id`.
    pub fn get_note(
        &self,
        ctx: &ModuleContext,
        note_id: &str,
    ) -> CoreResult<Option<StickyNoteRecord>> {
        ctx.db.with_conn(|conn| {
            conn.query_row(
                "SELECT id, title, content_md, theme_color, opacity,
                        pos_x, pos_y, width, height, is_pinned, is_open, updated_at
                 FROM sticky_notes
                 WHERE id = ?1",
                params![note_id],
                Self::map_row,
            )
            .optional()
            .map_err(CoreError::from)
        })
    }

    /// Lists all sticky notes (or only currently open desktop notes when `only_open = true`).
    pub fn list_notes(
        &self,
        ctx: &ModuleContext,
        only_open: bool,
    ) -> CoreResult<Vec<StickyNoteRecord>> {
        ctx.db.with_conn(|conn| {
            let sql = if only_open {
                "SELECT id, title, content_md, theme_color, opacity,
                        pos_x, pos_y, width, height, is_pinned, is_open, updated_at
                 FROM sticky_notes
                 WHERE is_open = 1
                 ORDER BY updated_at DESC"
            } else {
                "SELECT id, title, content_md, theme_color, opacity,
                        pos_x, pos_y, width, height, is_pinned, is_open, updated_at
                 FROM sticky_notes
                 ORDER BY is_open DESC, updated_at DESC"
            };
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt.query_map([], Self::map_row)?;
            let mut notes = Vec::new();
            for r in rows {
                notes.push(r?);
            }
            Ok(notes)
        })
    }

    /// Saves Markdown + LaTeX content for a sticky note (`sticky_save_content` IPC).
    pub fn save_content(
        &self,
        ctx: &ModuleContext,
        note_id: &str,
        title: Option<&str>,
        content_md: &str,
    ) -> CoreResult<StickyNoteRecord> {
        let now = Utc::now().timestamp_millis();
        ctx.db.with_conn(|conn| {
            let affected = if let Some(new_title) = title {
                conn.execute(
                    "UPDATE sticky_notes
                     SET title = ?1, content_md = ?2, updated_at = ?3
                     WHERE id = ?4",
                    params![new_title, content_md, now, note_id],
                )?
            } else {
                conn.execute(
                    "UPDATE sticky_notes
                     SET content_md = ?1, updated_at = ?2
                     WHERE id = ?3",
                    params![content_md, now, note_id],
                )?
            };

            if affected == 0 {
                return Err(CoreError::Internal(anyhow::anyhow!(
                    "Sticky note '{}' not found",
                    note_id
                )));
            }
            Ok(())
        })?;

        self.get_note(ctx, note_id)?
            .ok_or_else(|| CoreError::Internal(anyhow::anyhow!("Sticky note '{}' disappeared", note_id)))
    }

    /// Updates window geometry, opacity, and topmost pin state (`sticky_update_window_state` IPC).
    pub fn update_window_state(
        &self,
        ctx: &ModuleContext,
        note_id: &str,
        state: StickyNoteWindowStateUpdate,
    ) -> CoreResult<()> {
        let now = Utc::now().timestamp_millis();
        let clamped_opacity = state.opacity.clamp(0.2, 1.0);
        let width = state.width.max(220);
        let height = state.height.max(180);

        ctx.db.with_conn(|conn| {
            conn.execute(
                "UPDATE sticky_notes
                 SET pos_x = ?1, pos_y = ?2, width = ?3, height = ?4,
                     is_pinned = ?5, opacity = ?6, updated_at = ?7
                 WHERE id = ?8",
                params![
                    state.pos_x,
                    state.pos_y,
                    width,
                    height,
                    if state.is_pinned { 1 } else { 0 },
                    clamped_opacity,
                    now,
                    note_id
                ],
            )?;
            Ok(())
        })?;

        let label = Self::window_label_for(note_id);
        if ctx.window_pool.get_active_window(&label).is_some() {
            ctx.window_pool.set_always_on_top(&label, state.is_pinned)?;
        }
        Ok(())
    }

    /// Marks a sticky note as closed (`is_open = 0`) and immediately destroys its WebView window.
    pub fn close_note(&self, ctx: &ModuleContext, note_id: &str) -> CoreResult<()> {
        let now = Utc::now().timestamp_millis();
        ctx.db.with_conn(|conn| {
            conn.execute(
                "UPDATE sticky_notes SET is_open = 0, updated_at = ?1 WHERE id = ?2",
                params![now, note_id],
            )?;
            Ok(())
        })?;

        let label = Self::window_label_for(note_id);
        ctx.window_pool.destroy_window(&label)?;
        Ok(())
    }

    /// Re-opens an existing sticky note onto the desktop (`is_open = 1`) and spawns its `PinnedResident` WebView window.
    pub fn open_note(&self, ctx: &ModuleContext, note_id: &str) -> CoreResult<StickyNoteRecord> {
        let now = Utc::now().timestamp_millis();
        ctx.db.with_conn(|conn| {
            let affected = conn.execute(
                "UPDATE sticky_notes SET is_open = 1, updated_at = ?1 WHERE id = ?2",
                params![now, note_id],
            )?;
            if affected == 0 {
                return Err(CoreError::Internal(anyhow::anyhow!(
                    "Sticky note '{}' not found",
                    note_id
                )));
            }
            Ok(())
        })?;

        let record = self
            .get_note(ctx, note_id)?
            .ok_or_else(|| CoreError::Internal(anyhow::anyhow!("Sticky note '{}' not found", note_id)))?;
        self.spawn_note_window(ctx, &record)?;
        Ok(record)
    }

    /// Permanently deletes a sticky note from SQLite and destroys its WebView window if open.
    pub fn delete_note(&self, ctx: &ModuleContext, note_id: &str) -> CoreResult<bool> {
        let label = Self::window_label_for(note_id);
        let _ = ctx.window_pool.destroy_window(&label);

        ctx.db.with_conn(|conn| {
            let deleted =
                conn.execute("DELETE FROM sticky_notes WHERE id = ?1", params![note_id])?;
            Ok(deleted > 0)
        })
    }

    fn spawn_note_window(&self, ctx: &ModuleContext, note: &StickyNoteRecord) -> CoreResult<()> {
        let mut spec = WindowSpec::sticky_note(
            &note.id,
            note.pos_x.map(f64::from),
            note.pos_y.map(f64::from),
            note.width as f64,
            note.height as f64,
        );
        spec.always_on_top = note.is_pinned;
        spec.title = note.title.clone();
        ctx.window_pool.open_window(spec)
    }

    fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StickyNoteRecord> {
        let is_pinned_int: i64 = row.get(9)?;
        let is_open_int: i64 = row.get(10)?;
        Ok(StickyNoteRecord {
            id: row.get(0)?,
            title: row.get(1)?,
            content_md: row.get(2)?,
            theme_color: row.get(3)?,
            opacity: row.get(4)?,
            pos_x: row.get(5)?,
            pos_y: row.get(6)?,
            width: row.get(7)?,
            height: row.get(8)?,
            is_pinned: is_pinned_int != 0,
            is_open: is_open_int != 0,
            updated_at: row.get(11)?,
        })
    }
}

impl AppModule for StickyNoteModule {
    fn id(&self) -> &'static str {
        "sticky_note"
    }

    fn name(&self) -> &'static str {
        "桌面置顶富文本便笺 (Markdown + LaTeX)"
    }

    fn migrations(&self) -> Vec<ModuleMigration> {
        vec![ModuleMigration::new(
            "sticky_note",
            1,
            "Create sticky_notes table for Markdown + LaTeX sticky notes",
            "CREATE TABLE IF NOT EXISTS sticky_notes (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL DEFAULT '',
                content_md TEXT NOT NULL DEFAULT '',
                theme_color TEXT NOT NULL DEFAULT 'amber',
                opacity REAL NOT NULL DEFAULT 0.95,
                pos_x INTEGER,
                pos_y INTEGER,
                width INTEGER NOT NULL DEFAULT 320,
                height INTEGER NOT NULL DEFAULT 300,
                is_pinned INTEGER NOT NULL DEFAULT 1,
                is_open INTEGER NOT NULL DEFAULT 1,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_sticky_notes_open ON sticky_notes(is_open, updated_at DESC);",
        )]
    }

    fn default_shortcuts(&self) -> Vec<ShortcutSpec> {
        vec![ShortcutSpec::new(
            "sticky_note",
            "sticky_note:create",
            "PrimaryMod+Shift+N",
            "新建桌面置顶 Markdown + LaTeX 便笺",
        )]
    }

    fn on_init(&self, ctx: &ModuleContext) -> CoreResult<()> {
        // Restore only notes that were left open on desktop (`is_open = 1`).
        // If 0 notes are open, 0 WebView windows are created (preserving Level 0 memory budget).
        let open_notes = self.list_notes(ctx, true)?;
        for note in &open_notes {
            self.spawn_note_window(ctx, note)?;
        }
        info!(
            restored_open_notes = open_notes.len(),
            "StickyNoteModule initialized"
        );
        Ok(())
    }

    fn on_shortcut(&self, action_id: &str, ctx: &ModuleContext) -> CoreResult<()> {
        if action_id == "sticky_note:create" {
            self.create_note(ctx, Some("新建便笺"), Some(""), Some("amber"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_core::{AppKernelBuilder, TargetOs};

    #[test]
    fn test_sticky_note_lifecycle_and_zero_webview_when_closed() {
        let temp_dir = tempfile::tempdir().unwrap();
        let sticky = StickyNoteModule::new();

        // 1. First boot: 0 open notes -> 0 WebViews on startup
        let kernel = AppKernelBuilder::new(temp_dir.path())
            .with_target_os(TargetOs::MacOs)
            .register_module(sticky.clone())
            .unwrap()
            .bootstrap()
            .unwrap();

        let ctx = kernel.context();
        assert_eq!(ctx.window_pool.active_window_count(), 0);

        // 2. Create a note with Markdown + LaTeX formula
        let note = sticky
            .create_note(
                ctx,
                Some("物理公式备忘"),
                Some("质能方程：$E = mc^2$\n\n$$\\int_{-\\infty}^{\\infty} e^{-x^2} dx = \\sqrt{\\pi}$$"),
                Some("sky"),
            )
            .unwrap();
        assert_eq!(ctx.window_pool.active_window_count(), 1);

        // 3. Update window state and content
        sticky
            .update_window_state(
                ctx,
                &note.id,
                StickyNoteWindowStateUpdate {
                    pos_x: Some(400),
                    pos_y: Some(260),
                    width: 380,
                    height: 340,
                    is_pinned: false,
                    opacity: 0.88,
                },
            )
            .unwrap();

        let updated = sticky
            .save_content(ctx, &note.id, Some("更新后的标题"), "# 标题\n- [x] 已完成")
            .unwrap();
        assert_eq!(updated.title, "更新后的标题");
        assert!(!updated.is_pinned);
        assert_eq!(updated.width, 380);

        // 4. Close the note -> destroys WebView immediately (back to 0 active WebViews)
        sticky.close_note(ctx, &note.id).unwrap();
        assert_eq!(ctx.window_pool.active_window_count(), 0);
        kernel.shutdown().unwrap();
    }
}
