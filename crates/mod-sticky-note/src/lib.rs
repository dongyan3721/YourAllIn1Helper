//! `mod-sticky-note` —— 子功能 04：桌面置顶富文本便笺 (Markdown + LaTeX)
//!
//! 核心设计与特性：
//! - **Level 2 钉住驻留窗口生命周期**：在 `on_init` 初始化时，仅查询 `sticky_notes` 中 `is_open = 1` 的便笺并恢复原生窗口；
//!   若桌面无打开便笺，启动时保持 0 WebView 常驻；
//! - **实时与防抖持久化**：便笺标题、Markdown 内容、LaTeX 公式、主题色、透明度、窗口几何坐标与置顶状态统一持久化至 SQLite；
//! - **关闭即销毁释放内存**：用户点击便笺关闭按钮时，将 `is_open` 标记为 0，并立即调用 `window_pool.destroy_window("sticky-note-{id}")` 销毁 WebView 进程；
//! - **Markdown + LaTeX 导出**：支持一键将便笺内容导出为标准 `.md` 文件或格式化文本。

use app_core::{
    AppModule, CoreError, CoreResult, ModuleContext, ModuleMigration, ShortcutSpec, WindowSpec,
};
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

/// 桌面便笺持久化数据模型（对应 SQLite `sticky_notes` 表）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StickyNoteRecord {
    /// 便笺全局唯一 UUID 字符串
    pub id: String,
    /// 便笺标题
    pub title: String,
    /// Markdown 与 LaTeX 混合正文内容
    pub content_md: String,
    /// 主题色彩方案（如 'amber', 'sky', 'emerald', 'rose', 'violet', 'dark'）
    pub theme_color: String,
    /// 窗口透明度 (0.2 ~ 1.0)
    pub opacity: f64,
    /// 屏幕 X 像素物理坐标
    pub pos_x: Option<i32>,
    /// 屏幕 Y 像素物理坐标
    pub pos_y: Option<i32>,
    /// 窗口宽度（像素）
    pub width: u32,
    /// 窗口高度（像素）
    pub height: u32,
    /// 是否始终置顶最前 (Always On Top)
    pub is_pinned: bool,
    /// 是否在桌面上打开显示 (0: 已关闭/归档, 1: 桌面打开中)
    pub is_open: bool,
    /// 最近更新时间戳（毫秒）
    pub updated_at: i64,
}

/// 接收来自前端原生窗口拖拽调整大小、移动位置、置顶或透明度修改的同步载荷
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StickyNoteWindowStateUpdate {
    /// 最新屏幕 X 坐标
    pub pos_x: Option<i32>,
    /// 最新屏幕 Y 坐标
    pub pos_y: Option<i32>,
    /// 最新窗口宽度
    pub width: u32,
    /// 最新窗口高度
    pub height: u32,
    /// 最新置顶状态
    pub is_pinned: bool,
    /// 最新透明度
    pub opacity: f64,
}

/// 便笺模块实现
#[derive(Default, Clone)]
pub struct StickyNoteModule;

impl StickyNoteModule {
    /// 创建便笺模块实例
    pub fn new() -> Self {
        Self
    }

    /// 根据便笺 ID 生成规范的 WebView 原生窗口 Label
    pub fn window_label_for(note_id: &str) -> String {
        format!("sticky-note-{}", note_id)
    }

    /// 创建一个新便笺并立即在桌面上打开其独立的 Level 2 原生 WebView 窗口
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

    /// 获取单条便笺完整记录
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

    /// 查询便笺列表（`only_open = true` 时仅返回当前在桌面打开的便笺）
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

    /// 保存便笺标题与 Markdown 内容（前端 500ms 防抖调用）
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
                    "未找到指定的便笺 '{}'",
                    note_id
                )));
            }
            Ok(())
        })?;

        self.get_note(ctx, note_id)?.ok_or_else(|| {
            CoreError::Internal(anyhow::anyhow!("便笺 '{}' 更新后读取异常", note_id))
        })
    }

    /// 更新便笺窗口大小、坐标位置、透明度与置顶状态
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

    /// 关闭便笺（标记 `is_open = 0` 并立即销毁对应的 WebView 窗口归还系统内存）
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

    /// 重新在桌面唤醒已关闭的便笺（`is_open = 1` 并创建 WebView 窗口）
    pub fn open_note(&self, ctx: &ModuleContext, note_id: &str) -> CoreResult<StickyNoteRecord> {
        let now = Utc::now().timestamp_millis();
        ctx.db.with_conn(|conn| {
            let affected = conn.execute(
                "UPDATE sticky_notes SET is_open = 1, updated_at = ?1 WHERE id = ?2",
                params![now, note_id],
            )?;
            if affected == 0 {
                return Err(CoreError::Internal(anyhow::anyhow!(
                    "未找到要打开的便笺 '{}'",
                    note_id
                )));
            }
            Ok(())
        })?;

        let record = self
            .get_note(ctx, note_id)?
            .ok_or_else(|| CoreError::Internal(anyhow::anyhow!("未找到便笺 '{}'", note_id)))?;
        self.spawn_note_window(ctx, &record)?;
        Ok(record)
    }

    /// 彻底从数据库删除便笺，若处于打开状态则销毁其原生窗口
    pub fn delete_note(&self, ctx: &ModuleContext, note_id: &str) -> CoreResult<bool> {
        let label = Self::window_label_for(note_id);
        let _ = ctx.window_pool.destroy_window(&label);

        ctx.db.with_conn(|conn| {
            let deleted =
                conn.execute("DELETE FROM sticky_notes WHERE id = ?1", params![note_id])?;
            Ok(deleted > 0)
        })
    }

    /// 将便笺格式化导出为 Markdown 字符串
    pub fn export_markdown(&self, ctx: &ModuleContext, note_id: &str) -> CoreResult<String> {
        let note = self
            .get_note(ctx, note_id)?
            .ok_or_else(|| CoreError::Internal(anyhow::anyhow!("未找到便笺 '{}'", note_id)))?;

        let date_str = chrono::DateTime::from_timestamp_millis(note.updated_at)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "未知时间".to_string());

        let md = format!(
            "# {}\n\n> 导出时间：{}\n> 便笺 ID：{}\n\n{}\n",
            note.title, date_str, note.id, note.content_md
        );
        Ok(md)
    }

    /// 派生并打开便笺对应的原生 WebView 窗口
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

    /// 将数据库行映射为强类型便笺结构体
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
            "创建 sticky_notes 便笺表与活跃查询复合索引",
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

    fn on_shutdown(&self, ctx: &ModuleContext) -> CoreResult<()> {
        // Keep is_open and content so enabling the block restores the same notes.
        for window in ctx.window_pool.list_active_windows() {
            if window.spec.label.starts_with("sticky-note-") {
                ctx.window_pool.destroy_window(&window.spec.label)?;
            }
        }
        Ok(())
    }

    fn on_init(&self, ctx: &ModuleContext) -> CoreResult<()> {
        // 启动时仅恢复桌面未关闭的便笺 (is_open = 1)
        // 若桌面无便笺，则创建 0 个 WebView 窗口，保持 Level 0 内存预算目标
        let open_notes = self.list_notes(ctx, true)?;
        for note in &open_notes {
            self.spawn_note_window(ctx, note)?;
        }
        info!(
            restored_open_notes = open_notes.len(),
            "便笺模块 StickyNoteModule 初始化就绪"
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
#[path = "../tests/unit/lib.rs"]
mod tests;
