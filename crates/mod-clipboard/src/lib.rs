//! `mod-clipboard` —— 子功能 02：Ditto 风格剪切板管理器
//!
//! 核心设计与特性：
//! - **SQLite `clipboard_items` 数据持久化**：统一存储文本与图片记录，基于 SHA-256 唯一哈希值排重；
//! - **内容排重与时间戳更新**：重复复制相同内容时，自动更新 `updated_at` 浮动到历史最顶端，不产生重复记录；
//! - **LRU-100 自动淘汰**：严格执行配置项 `config.clipboard.max_history_items`（默认 100 条）淘汰未固定项，已钉住（`is_pinned = 1`）的条目永久保留；
//! - **防污染屏蔽守卫（Anti-Pollution）**：在捕获前检查 `EventBus::is_clipboard_suppressed()`，自动屏蔽划词翻译模拟复制、密码本复制等敏感/内部操作；
//! - **图片真实落盘与缩略图**：自动将剪贴板图片落盘为 PNG 文件，并生成低开销 JPEG Base64 缩略图供前端秒开展示；
//! - **快速粘贴上屏**：支持文本与真实图片写回系统剪贴板，先销毁瞬态弹窗，再调用 `os_hal::InputSimulator::send_paste_shortcut()` 模拟粘贴。

use app_core::{
    AppModule, CoreError, CoreResult, ModuleContext, ModuleMigration, ShortcutSpec, SystemEvent,
    WindowSpec,
};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use chrono::Utc;
use image::{codecs::jpeg::JpegEncoder, imageops::FilterType, DynamicImage, ImageBuffer, Rgba};
use os_hal::InputSimulator;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, sync::Arc};
use tracing::info;

/// 剪贴板历史记录类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardItemType {
    /// 纯文本记录
    Text,
    /// 图片记录（包含本地落盘相对路径与 Base64 缩略图）
    Image,
}

impl ClipboardItemType {
    /// 转为数据库存储的字符串枚举
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
        }
    }

    /// 从数据库字符串映射为枚举类型
    pub fn from_db_str(s: &str) -> Self {
        match s {
            "image" => Self::Image,
            _ => Self::Text,
        }
    }
}

/// 返回给前端 UI（`clipboard-popup`）的单条剪贴板记录数据结构
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardRecord {
    /// 唯一主键自增 ID
    pub id: i64,
    /// 记录类型（文本/图片）
    pub item_type: ClipboardItemType,
    /// 文本内容（仅文本类型有值）
    pub content_text: Option<String>,
    /// 图片 blob 文件相对路径（仅图片类型有值，保存在 clipboard_blobs/ 目录下）
    pub blob_rel_path: Option<String>,
    /// 图片 Base64 缩略图（用于前端列表即时渲染）
    pub thumb_base64: Option<String>,
    /// SHA-256 去重哈希值
    pub content_hash: String,
    /// 复制来源应用程序名称（如 "Terminal", "VS Code" 等）
    pub source_app_name: Option<String>,
    /// 是否已固定（已固定条目不受 LRU 自动淘汰影响）
    pub is_pinned: bool,
    /// 首次创建时间戳（毫秒）
    pub created_at: i64,
    /// 最近更新/重新复制时间戳（毫秒）
    pub updated_at: i64,
}

/// 剪切板管理模块实现
pub struct ClipboardModule {
    /// 键盘与鼠标按键模拟器（用于模拟 Cmd+V / Ctrl+V）
    input_simulator: Arc<dyn InputSimulator>,
}

impl ClipboardModule {
    /// 创建剪贴板模块实例
    pub fn new(input_simulator: Arc<dyn InputSimulator>) -> Self {
        Self { input_simulator }
    }

    /// 计算前缀加字节流的 SHA-256 十六进制哈希值，作为全局去重唯一键
    pub fn compute_hash(prefix: &str, bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(prefix.as_bytes());
        hasher.update(b":");
        hasher.update(bytes);
        format!("{:x}", hasher.finalize())
    }

    /// 捕获并存储文本条目，支持去重与 LRU 自动淘汰
    /// 若当前事件总线处于防污染屏蔽状态或文本为空，则直接返回 `Ok(None)`
    pub fn capture_text(
        &self,
        ctx: &ModuleContext,
        text: &str,
        source_app_name: Option<&str>,
    ) -> CoreResult<Option<ClipboardRecord>> {
        // 检查防污染标记：密码本复制或划词翻译模拟复制时直接跳过
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

            // 检查数据库中是否已存在相同哈希的记录
            let existing_id: Option<i64> = tx
                .query_row(
                    "SELECT id FROM clipboard_items WHERE content_hash = ?1",
                    params![content_hash],
                    |row| row.get(0),
                )
                .optional()?;

            let record_id = if let Some(id) = existing_id {
                // 已存在相同内容：仅更新 updated_at 与可能补充的来源应用名，使其浮动到最顶部
                tx.execute(
                    "UPDATE clipboard_items
                     SET updated_at = ?1, source_app_name = COALESCE(?2, source_app_name)
                     WHERE id = ?3",
                    params![effective_now, source_app_name, id],
                )?;
                id
            } else {
                // 新内容：插入新记录
                tx.execute(
                    "INSERT INTO clipboard_items (
                        item_type, content_text, blob_rel_path, thumb_base64,
                        content_hash, source_app_name, is_pinned, created_at, updated_at
                    ) VALUES ('text', ?1, NULL, NULL, ?2, ?3, 0, ?4, ?4)",
                    params![text, content_hash, source_app_name, effective_now],
                )?;
                tx.last_insert_rowid()
            };

            // 执行 LRU 淘汰：删除超出容量限制的最旧未固定记录
            Self::evict_lru_unpinned(&tx, max_items)?;
            tx.commit()?;
            Ok(record_id)
        })?;

        // 向内部事件总线发布更新事件，通知 UI 刷新
        ctx.event_bus.publish(SystemEvent::ClipboardUpdated {
            item_id,
            item_type: "text".to_string(),
        });

        self.get_item_by_id(ctx, item_id)
    }

    /// 捕获并存储原始 RGBA 图片条目（全流程真实落盘与缩略图生成）
    pub fn capture_image_rgba(
        &self,
        ctx: &ModuleContext,
        width: u32,
        height: u32,
        rgba_bytes: &[u8],
        source_app_name: Option<&str>,
    ) -> CoreResult<Option<ClipboardRecord>> {
        // 检查防污染标记
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
        if !enabled || !capture_images || width == 0 || height == 0 || rgba_bytes.is_empty() {
            return Ok(None);
        }

        // 计算图片像素哈希
        let content_hash = Self::compute_hash("image", rgba_bytes);
        let now = Utc::now().timestamp_millis();

        // 构建 DynamicImage 对象
        let rgba_image: ImageBuffer<Rgba<u8>, Vec<u8>> =
            ImageBuffer::from_raw(width, height, rgba_bytes.to_vec()).ok_or_else(|| {
                CoreError::Internal(anyhow::anyhow!("无法从 RGBA 字节流构建图片缓冲区"))
            })?;
        let dynamic = DynamicImage::ImageRgba8(rgba_image);

        // 获取或创建 blobs 存储目录
        let blobs_dir = ctx.app_data_dir.join("clipboard_blobs");
        fs::create_dir_all(&blobs_dir)?;

        // 保存完整 PNG 文件
        let file_name = format!("{}.png", content_hash);
        let blob_full_path = blobs_dir.join(&file_name);
        if !blob_full_path.exists() {
            dynamic
                .save_with_format(&blob_full_path, image::ImageFormat::Png)
                .map_err(|e| CoreError::Internal(anyhow::anyhow!("保存剪贴板 PNG 失败: {}", e)))?;
        }
        let blob_rel_path = format!("clipboard_blobs/{}", file_name);

        // 生成不超过 240px 的高质量 Base64 缩略图
        let thumb_base64 = Self::generate_thumbnail(&dynamic, 240)?;

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

    /// 兼容旧接口的图片捕获
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

    /// 查询剪贴板历史记录，按“固定项置顶、最近更新时间倒序”排列
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

    /// 根据唯一 ID 获取单条剪贴板记录
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

    /// 切换条目的固定状态（已固定项豁免 LRU 淘汰）
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

    /// 删除单条剪贴板记录（若为图片则同步尝试清理 blob 文件）
    pub fn delete_item(&self, ctx: &ModuleContext, id: i64) -> CoreResult<bool> {
        let record = self.get_item_by_id(ctx, id)?;
        let affected = ctx.db.with_conn(|conn| {
            let n = conn.execute("DELETE FROM clipboard_items WHERE id = ?1", params![id])?;
            Ok(n)
        })?;

        // 清理本地关联的 blob 图片
        if let Some(rec) = record {
            if let Some(rel) = rec.blob_rel_path {
                let full = ctx.app_data_dir.join(rel);
                let _ = fs::remove_file(full);
            }
        }
        Ok(affected > 0)
    }

    /// 清空所有未固定的历史记录（永久保留已固定项）
    pub fn clear_unpinned(&self, ctx: &ModuleContext) -> CoreResult<usize> {
        // 先查询所有将被删除的未固定图片的相对路径
        let blobs_to_delete: Vec<String> = ctx.db.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT blob_rel_path FROM clipboard_items
                 WHERE is_pinned = 0 AND blob_rel_path IS NOT NULL",
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            let mut paths = Vec::new();
            for r in rows {
                if let Ok(p) = r {
                    paths.push(p);
                }
            }
            Ok(paths)
        })?;

        let deleted = ctx.db.with_conn(|conn| {
            let count = conn.execute("DELETE FROM clipboard_items WHERE is_pinned = 0", [])?;
            Ok(count)
        })?;

        for rel in blobs_to_delete {
            let full = ctx.app_data_dir.join(rel);
            let _ = fs::remove_file(full);
        }

        Ok(deleted)
    }

    /// 快速粘贴选中条目到前台目标窗口
    /// 先立即销毁瞬态 `clipboard-popup` 原生窗口，更新最近使用时间，然后模拟按键粘贴
    pub fn paste_item(
        &self,
        ctx: &ModuleContext,
        id: i64,
        _as_plain_text: bool,
    ) -> CoreResult<ClipboardRecord> {
        let record = self
            .get_item_by_id(ctx, id)?
            .ok_or_else(|| CoreError::Internal(anyhow::anyhow!("未找到剪贴板条目 {}", id)))?;

        // 模拟按键前先彻底销毁瞬态弹窗，将焦点归还给原前台应用窗口
        let _ = ctx.window_pool.destroy_window("clipboard-popup");

        // 刷新最近使用时间戳，使刚粘贴的项立即置顶
        let now = Utc::now().timestamp_millis();
        ctx.db.with_conn(|conn| {
            let effective_now = Self::next_monotonic_timestamp(conn, now)?;
            conn.execute(
                "UPDATE clipboard_items SET updated_at = ?1 WHERE id = ?2",
                params![effective_now, id],
            )?;
            Ok(())
        })?;

        // 触发物理按键模拟（Cmd+V / Ctrl+V）
        self.input_simulator
            .send_paste_shortcut()
            .map_err(|e| CoreError::Internal(anyhow::anyhow!(e)))?;

        Ok(record)
    }

    /// 保证时间戳单调递增，避免毫秒级冲突
    fn next_monotonic_timestamp(conn: &rusqlite::Connection, now: i64) -> CoreResult<i64> {
        let max_ts: i64 = conn.query_row(
            "SELECT COALESCE(MAX(updated_at), 0) FROM clipboard_items",
            [],
            |row| row.get(0),
        )?;
        Ok(now.max(max_ts + 1))
    }

    /// 淘汰未固定的旧记录
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

    /// 将数据库行映射为强类型记录结构体
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

    /// 生成轻量 JPEG Base64 缩略图
    fn generate_thumbnail(dynamic: &DynamicImage, max_dim: u32) -> CoreResult<String> {
        let thumb = if dynamic.width() > max_dim || dynamic.height() > max_dim {
            dynamic.resize(max_dim, max_dim, FilterType::Triangle)
        } else {
            dynamic.clone()
        };
        let mut buf = Vec::new();
        let mut encoder = JpegEncoder::new_with_quality(&mut buf, 75);
        encoder
            .encode_image(&thumb)
            .map_err(|e| CoreError::Internal(anyhow::anyhow!("缩略图编码失败: {}", e)))?;
        Ok(format!(
            "data:image/jpeg;base64,{}",
            BASE64_STANDARD.encode(buf)
        ))
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
            "创建 clipboard_items 剪贴板表及 LRU 复合索引",
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

    fn on_shutdown(&self, ctx: &ModuleContext) -> CoreResult<()> {
        ctx.window_pool.destroy_window("clipboard-popup")?;
        Ok(())
    }

    fn on_init(&self, _ctx: &ModuleContext) -> CoreResult<()> {
        info!("剪切板模块 mod-clipboard 初始化就绪 (后台守护监听，0 常驻 WebView)");
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
#[path = "../tests/unit/lib.rs"]
mod tests;
