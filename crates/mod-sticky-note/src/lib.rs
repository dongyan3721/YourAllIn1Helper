//! `mod-sticky-note` —— 子功能 04：桌面置顶富文本便笺 (Markdown + LaTeX)
//!
//! 核心设计与特性：
//! - **Level 2 钉住驻留窗口生命周期**：在 `on_init` 初始化时，仅查询 `sticky_notes` 中 `is_open = 1` 的便笺并恢复原生窗口；
//!   若桌面无打开便笺，启动时保持 0 WebView 常驻；
//! - **实时与防抖持久化**：便笺标题、Markdown 内容、LaTeX 公式、主题色、透明度、窗口几何坐标与置顶状态统一持久化至 SQLite；
//!   前端在关闭前冲刷尚未落盘的标题、正文和窗口状态。每次打开页面获得一个新的写入会话；
//!   同一会话内序号更旧的写入不会覆盖已提交内容，旧会话的延迟写入也不会覆盖新会话；
//! - **关闭即销毁释放内存**：页面确认保存成功后，将 `is_open` 标记为 0，并调用 `window_pool.destroy_window("sticky-note-{id}")` 销毁 WebView 进程；
//! - **Markdown + LaTeX 导出**：支持一键将便笺内容导出为标准 `.md` 文件或格式化文本。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

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

/// 一次标题、主题或窗口写入的结果。`applied == false` 表示序号或会话被拒绝，数据库保持原值。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StickyNoteWrite {
    /// 这次请求是否真正落盘。
    pub applied: bool,
    /// 拒绝时是当前记录，接受时是写入后的记录。
    #[serde(flatten)]
    pub note: StickyNoteRecord,
}

impl std::ops::Deref for StickyNoteWrite {
    type Target = StickyNoteRecord;

    fn deref(&self) -> &Self::Target {
        &self.note
    }
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

/// 一次 WebView 打开期间的写入游标。`session_id == 0` 表示尚未显式开会话的旧调用。
struct NoteWriteCursor {
    session_id: u64,
    content_seq: u64,
    theme_seq: u64,
    window_seq: u64,
}

impl NoteWriteCursor {
    fn legacy() -> Self {
        Self {
            session_id: 0,
            content_seq: 0,
            theme_seq: 0,
            window_seq: 0,
        }
    }

    fn for_session(session_id: u64) -> Self {
        Self {
            session_id,
            content_seq: 0,
            theme_seq: 0,
            window_seq: 0,
        }
    }
}

#[derive(Clone, Copy)]
enum WriteField {
    Content,
    Theme,
    Window,
}

struct WriteGate {
    apply: bool,
    created: bool,
}

/// 同一把锁覆盖一次打开的会话号，以及标题、主题、窗口三类序号。
struct WriteBook {
    notes: Mutex<HashMap<String, NoteWriteCursor>>,
    next_session: AtomicU64,
}

/// 便笺模块实现
#[derive(Clone)]
pub struct StickyNoteModule {
    writes: Arc<WriteBook>,
}

impl Default for StickyNoteModule {
    fn default() -> Self {
        Self {
            writes: Arc::new(WriteBook {
                notes: Mutex::new(HashMap::new()),
                next_session: AtomicU64::new(0),
            }),
        }
    }
}

impl StickyNoteModule {
    /// 创建便笺模块实例
    pub fn new() -> Self {
        Self::default()
    }

    fn lock_writes(&self) -> std::sync::MutexGuard<'_, HashMap<String, NoteWriteCursor>> {
        self.writes
            .notes
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }

    fn field_seq(cursor: &NoteWriteCursor, field: WriteField) -> u64 {
        match field {
            WriteField::Content => cursor.content_seq,
            WriteField::Theme => cursor.theme_seq,
            WriteField::Window => cursor.window_seq,
        }
    }

    fn set_field_seq(cursor: &mut NoteWriteCursor, field: WriteField, seq: u64) {
        match field {
            WriteField::Content => cursor.content_seq = seq,
            WriteField::Theme => cursor.theme_seq = seq,
            WriteField::Window => cursor.window_seq = seq,
        }
    }

    /// 决定这次写入能否落盘。会话号在锁内分配和比较，旧会话不能插到新会话前面。
    fn prepare_write(
        writes: &mut HashMap<String, NoteWriteCursor>,
        note_id: &str,
        session_id: Option<u64>,
        write_seq: Option<u64>,
        field: WriteField,
    ) -> WriteGate {
        let current = writes
            .get(note_id)
            .map(|cursor| (cursor.session_id, Self::field_seq(cursor, field)));
        match (session_id, write_seq) {
            (None, None) => WriteGate {
                apply: current.map(|(session, _)| session == 0).unwrap_or(true),
                created: false,
            },
            (Some(_), None) => WriteGate {
                apply: false,
                created: false,
            },
            (None, Some(seq)) => match current {
                Some((session, _)) if session != 0 => WriteGate {
                    apply: false,
                    created: false,
                },
                Some((_, stored)) => WriteGate {
                    apply: seq >= stored,
                    created: false,
                },
                None => {
                    writes.insert(note_id.to_string(), NoteWriteCursor::legacy());
                    WriteGate {
                        apply: true,
                        created: true,
                    }
                }
            },
            (Some(session), Some(seq)) => match current {
                Some((current_session, _)) if current_session != session => WriteGate {
                    apply: false,
                    created: false,
                },
                Some((_, stored)) => WriteGate {
                    apply: seq >= stored,
                    created: false,
                },
                None => {
                    writes.insert(note_id.to_string(), NoteWriteCursor::for_session(session));
                    WriteGate {
                        apply: true,
                        created: true,
                    }
                }
            },
        }
    }

    fn commit_write(
        writes: &mut HashMap<String, NoteWriteCursor>,
        note_id: &str,
        session_id: Option<u64>,
        write_seq: Option<u64>,
        field: WriteField,
    ) {
        let Some(seq) = write_seq else {
            return;
        };
        let Some(cursor) = writes.get_mut(note_id) else {
            return;
        };
        let session_ok = match session_id {
            Some(session) => cursor.session_id == session,
            None => cursor.session_id == 0,
        };
        if session_ok && seq >= Self::field_seq(cursor, field) {
            Self::set_field_seq(cursor, field, seq);
        }
    }

    fn abandon_created(
        writes: &mut HashMap<String, NoteWriteCursor>,
        note_id: &str,
        created: bool,
    ) {
        if created {
            writes.remove(note_id);
        }
    }

    fn read_required(&self, ctx: &ModuleContext, note_id: &str) -> CoreResult<StickyNoteRecord> {
        self.get_note(ctx, note_id)?.ok_or_else(|| {
            CoreError::Internal(anyhow::anyhow!("便笺 '{}' 更新后读取异常", note_id))
        })
    }

    fn write_result(
        &self,
        ctx: &ModuleContext,
        note_id: &str,
        applied: bool,
    ) -> CoreResult<StickyNoteWrite> {
        Ok(StickyNoteWrite {
            applied,
            note: self.read_required(ctx, note_id)?,
        })
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

    /// 为一次新打开的 WebView 分配写入会话，并把该便笺的三类序号清零。
    ///
    /// 会话号在进程内递增。旧会话此后即使带着更大的 `write_seq` 也不能覆盖新会话。
    pub fn begin_write_session(&self, ctx: &ModuleContext, note_id: &str) -> CoreResult<u64> {
        let mut writes = self.lock_writes();
        let exists = ctx.db.with_conn(|conn| {
            conn.query_row(
                "SELECT 1 FROM sticky_notes WHERE id = ?1",
                params![note_id],
                |_| Ok(()),
            )
            .optional()
            .map_err(CoreError::from)
        })?;
        if exists.is_none() {
            return Err(CoreError::Internal(anyhow::anyhow!(
                "未找到要开始写入会话的便笺 '{}'",
                note_id
            )));
        }
        let session_id = self.writes.next_session.fetch_add(1, Ordering::Relaxed) + 1;
        writes.insert(
            note_id.to_string(),
            NoteWriteCursor {
                session_id,
                content_seq: 0,
                theme_seq: 0,
                window_seq: 0,
            },
        );
        Ok(session_id)
    }

    /// 保存便笺标题与 Markdown 内容。
    ///
    /// `write_seq` 在同一个 `session_id` 内按发送顺序比较。序号更小的请求不会覆盖已提交内容；
    /// 相等序号允许重试。显式会话开始后，不带会话或属于旧会话的写入都不会覆盖它。
    /// 两者都为 `None` 时保持旧调用方的直接写入行为。
    /// 返回的 `applied` 标明这次请求是落盘还是被拒绝，拒绝不会伪装成成功。
    pub fn save_content(
        &self,
        ctx: &ModuleContext,
        note_id: &str,
        title: Option<&str>,
        content_md: &str,
        write_seq: Option<u64>,
        session_id: Option<u64>,
    ) -> CoreResult<StickyNoteWrite> {
        let mut writes = self.lock_writes();
        let gate = Self::prepare_write(
            &mut writes,
            note_id,
            session_id,
            write_seq,
            WriteField::Content,
        );
        if !gate.apply {
            drop(writes);
            return self.write_result(ctx, note_id, false);
        }

        let now = Utc::now().timestamp_millis();
        let saved = ctx.db.with_conn(|conn| {
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
        });
        if let Err(err) = saved {
            Self::abandon_created(&mut writes, note_id, gate.created);
            return Err(err);
        }
        Self::commit_write(
            &mut writes,
            note_id,
            session_id,
            write_seq,
            WriteField::Content,
        );
        drop(writes);

        self.write_result(ctx, note_id, true)
    }

    /// 更新便笺窗口大小、坐标位置、透明度与置顶状态。
    ///
    /// 过期序号、旧会话和不带会话的写入都不会改写数据库，也不会把窗口层级退回到旧的置顶状态。
    /// `applied == false` 表示调用方必须保留自己的待保存状态。
    pub fn update_window_state(
        &self,
        ctx: &ModuleContext,
        note_id: &str,
        state: StickyNoteWindowStateUpdate,
        write_seq: Option<u64>,
        session_id: Option<u64>,
    ) -> CoreResult<StickyNoteWrite> {
        let mut writes = self.lock_writes();
        let gate = Self::prepare_write(
            &mut writes,
            note_id,
            session_id,
            write_seq,
            WriteField::Window,
        );
        if !gate.apply {
            drop(writes);
            return self.write_result(ctx, note_id, false);
        }

        let now = Utc::now().timestamp_millis();
        let clamped_opacity = state.opacity.clamp(0.2, 1.0);
        let width = state.width.max(220);
        let height = state.height.max(180);

        let saved = ctx.db.with_conn(|conn| {
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
        });
        if let Err(err) = saved {
            Self::abandon_created(&mut writes, note_id, gate.created);
            return Err(err);
        }
        Self::commit_write(
            &mut writes,
            note_id,
            session_id,
            write_seq,
            WriteField::Window,
        );
        let label = Self::window_label_for(note_id);
        if ctx.window_pool.get_active_window(&label).is_some() {
            ctx.window_pool.set_always_on_top(&label, state.is_pinned)?;
        }
        drop(writes);
        self.write_result(ctx, note_id, true)
    }

    /// 更新便笺主题色。过期序号和旧会话都保留已经提交的颜色。
    /// `applied == false` 表示颜色没有改变，调用方不能把这次请求当成已保存。
    pub fn update_theme(
        &self,
        ctx: &ModuleContext,
        note_id: &str,
        theme_color: &str,
        write_seq: Option<u64>,
        session_id: Option<u64>,
    ) -> CoreResult<StickyNoteWrite> {
        let mut writes = self.lock_writes();
        let gate = Self::prepare_write(
            &mut writes,
            note_id,
            session_id,
            write_seq,
            WriteField::Theme,
        );
        if !gate.apply {
            drop(writes);
            return self.write_result(ctx, note_id, false);
        }

        let saved = ctx.db.with_conn(|conn| {
            conn.execute(
                "UPDATE sticky_notes SET theme_color = ?1 WHERE id = ?2",
                params![theme_color, note_id],
            )?;
            Ok(())
        });
        if let Err(err) = saved {
            Self::abandon_created(&mut writes, note_id, gate.created);
            return Err(err);
        }
        Self::commit_write(
            &mut writes,
            note_id,
            session_id,
            write_seq,
            WriteField::Theme,
        );
        drop(writes);
        self.write_result(ctx, note_id, true)
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
        let deleted = {
            let mut writes = self.lock_writes();
            let deleted = ctx.db.with_conn(|conn| {
                let deleted =
                    conn.execute("DELETE FROM sticky_notes WHERE id = ?1", params![note_id])?;
                Ok(deleted > 0)
            })?;
            writes.remove(note_id);
            deleted
        };
        let _ = ctx.window_pool.destroy_window(&label);
        Ok(deleted)
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
