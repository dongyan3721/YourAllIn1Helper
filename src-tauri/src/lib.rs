//! `all-in-one-host` (`src-tauri`) — Microkernel Desktop Host & IPC Bridge.
//!
//! Assembles the `AppKernel` daemon and mounts the 3 Phase-1 feature modules:
//! 1. **Module 02 (`mod-clipboard`)**: Ditto-style SHA-256 deduplicated LRU-100 clipboard manager.
//! 2. **Module 04 (`mod-sticky-note`)**: Pinned desktop Markdown + LaTeX rich-text sticky notes.
//! 3. **Module 09 (`mod-window-pin`)**: DeskPins-style external window topmost controller.
//!
//! Exposes a unified JSON IPC command dispatcher (`DesktopHost::invoke_ipc`). Production uses
//! the real Tauri 2.x shell in `tauri_app.rs`; no Swift host or local HTTP server is started.

use app_core::{
    AppKernel, AppKernelBuilder, CoreError, CoreResult, ExternalToolKind, HeadlessWindowBackend,
    TargetOs, WindowBackend, WindowSpec,
};
use mod_clipboard::ClipboardModule;
use mod_sticky_note::{StickyNoteModule, StickyNoteWindowStateUpdate};
use mod_window_pin::WindowPinModule;
use os_hal::{ExternalWindowInfo, NativeOsHal};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::Arc,
};
use tracing::error;

mod tauri_app;
pub use tauri_app::run;

/// Request payload sent to `POST /api/ipc` or `DesktopHost::invoke_ipc`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcRequest {
    pub cmd: String,
    #[serde(default)]
    pub args: Value,
}

/// Standardized IPC response envelope returned to `src-ui`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcResponse {
    pub ok: bool,
    pub data: Option<Value>,
    pub error: Option<String>,
}

/// 将文本写回操作系统剪贴板
fn write_text_to_system_clipboard(text: &str) -> CoreResult<()> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|error| CoreError::Internal(anyhow::anyhow!(error)))?;
    clipboard
        .set_text(text.to_string())
        .map_err(|error| CoreError::Internal(anyhow::anyhow!(error)))
}

/// 将本地图片文件写回操作系统剪贴板
fn write_image_to_system_clipboard(image_path: &Path) -> CoreResult<()> {
    let img = image::open(image_path)
        .map_err(|e| CoreError::Internal(anyhow::anyhow!("打开剪贴板图片失败: {}", e)))?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let raw = rgba.into_raw();
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| CoreError::Internal(anyhow::anyhow!(e)))?;
    clipboard
        .set_image(arboard::ImageData {
            width: w as usize,
            height: h as usize,
            bytes: std::borrow::Cow::Borrowed(&raw),
        })
        .map_err(|e| CoreError::Internal(anyhow::anyhow!("写入系统剪贴板图片失败: {}", e)))?;
    Ok(())
}

impl IpcResponse {
    pub fn success(data: impl Serialize) -> Self {
        Self {
            ok: true,
            data: serde_json::to_value(data).ok(),
            error: None,
        }
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(msg.into()),
        }
    }
}

/// Assembled desktop host holding the running `AppKernel` and mounted feature module handles.
pub struct DesktopHost {
    kernel: AppKernel,
    clipboard: Arc<ClipboardModule>,
    sticky_note: Arc<StickyNoteModule>,
    window_pin: Arc<WindowPinModule>,
    os_hal: NativeOsHal,
}

impl DesktopHost {
    /// Bootstraps the `AppKernel` and mounts Modules 02 (`mod-clipboard`), 04 (`mod-sticky-note`),
    /// and 09 (`mod-window-pin`).
    pub fn bootstrap(app_data_dir: impl Into<PathBuf>, in_memory_db: bool) -> CoreResult<Self> {
        Self::bootstrap_with_hal(
            app_data_dir,
            in_memory_db,
            Arc::new(HeadlessWindowBackend),
            NativeOsHal::mock_only(),
        )
    }

    /// Bootstraps with native OS capabilities and a real window backend (Tauri in production).
    pub fn bootstrap_with_window_backend(
        app_data_dir: impl Into<PathBuf>,
        in_memory_db: bool,
        window_backend: Arc<dyn WindowBackend>,
    ) -> CoreResult<Self> {
        Self::bootstrap_with_hal(
            app_data_dir,
            in_memory_db,
            window_backend,
            NativeOsHal::new(),
        )
    }

    fn bootstrap_with_hal(
        app_data_dir: impl Into<PathBuf>,
        in_memory_db: bool,
        window_backend: Arc<dyn WindowBackend>,
        os_hal: NativeOsHal,
    ) -> CoreResult<Self> {
        if os_hal.is_mock_only() {
            // Add a third deterministic external window for DeskPins integration tests.
            os_hal.mock().upsert_window(ExternalWindowInfo {
                handle: 1003,
                title: "Safari — Tauri 2.x Architecture Docs".to_string(),
                process_name: "Safari".to_string(),
                process_id: 4203,
                x: 340,
                y: 120,
                width: 1120,
                height: 740,
                is_topmost: false,
            });
        }

        let hal_bundle = os_hal.clone().into_bundle();

        let clipboard_mod = ClipboardModule::new(Arc::clone(&hal_bundle.input_simulator));
        let sticky_mod = StickyNoteModule::new();
        let winpin_mod = WindowPinModule::new(Arc::clone(&hal_bundle.window_controller));

        let kernel = AppKernelBuilder::new(app_data_dir)
            .with_in_memory_db(in_memory_db)
            .with_target_os(TargetOs::current())
            .with_window_backend(window_backend)
            .register_module(ClipboardModule::new(Arc::clone(
                &hal_bundle.input_simulator,
            )))?
            .register_module(sticky_mod.clone())?
            .register_module(winpin_mod.clone())?
            .bootstrap()?;

        Ok(Self {
            kernel,
            clipboard: Arc::new(clipboard_mod),
            sticky_note: Arc::new(sticky_mod),
            window_pin: Arc::new(winpin_mod),
            os_hal,
        })
    }

    /// Seeds initial demo records if the database has no clipboard items or sticky notes yet,
    /// while preserving the Level 0 (0 active WebViews) invariant until the user opens a window
    /// or triggers a shortcut.
    pub fn seed_demo_data_if_empty(&self) -> CoreResult<()> {
        let ctx = self.kernel.context();
        let existing_clip = self.clipboard.list_items(ctx, None, 5)?;
        if existing_clip.is_empty() {
            let item1 = self.clipboard.capture_text(
                ctx,
                "cargo test --workspace --all-targets",
                Some("Terminal"),
            )?;
            if let Some(rec) = item1 {
                // Pin the command as a frequently used snippet
                let _ = self.clipboard.toggle_pin(ctx, rec.id);
            }
            let _ = self.clipboard.capture_text(
                ctx,
                "PrimaryMod+Shift+V 唤醒 Ditto 风格剪贴板弹窗（失焦或粘贴后立即销毁 WebView）",
                Some("Visual Studio Code"),
            )?;
            let _ = self.clipboard.capture_text(
                ctx,
                "https://v2.tauri.app/concept/inter-process-communication/",
                Some("Safari"),
            )?;
        }

        let existing_notes = self.sticky_note.list_notes(ctx, false)?;
        if existing_notes.is_empty() {
            // Create a sample Markdown + LaTeX sticky note and close it so startup remains at Level 0 (0 WebViews),
            // ready to be opened from Dashboard or created via PrimaryMod+Shift+N.
            let note = self.sticky_note.create_note(
                ctx,
                Some("架构指标与公式速记"),
                Some(
                    "### 核心指标\n- [x] 后台静默 **0 WebView** 常驻 (< 25 MB)\n- [x] SQLite WAL + 自动增量迁移\n\n### 数学公式 (LaTeX)\n行内公式：$E = mc^2$，高斯积分：\n$$\\int_{-\\infty}^{\\infty} e^{-x^2}\\,dx = \\sqrt{\\pi}$$",
                ),
                Some("amber"),
            )?;
            self.sticky_note.close_note(ctx, &note.id)?;
        }

        Ok(())
    }

    /// Returns a reference to the underlying `AppKernel`.
    pub fn kernel(&self) -> &AppKernel {
        &self.kernel
    }

    /// Dispatches a typed IPC command from the frontend (`src-ui`) or integration tests.
    pub fn invoke_ipc(&self, cmd: &str, args: Value) -> IpcResponse {
        match self.dispatch_inner(cmd, args) {
            Ok(val) => IpcResponse::success(val),
            Err(err) => IpcResponse::err(err.to_string()),
        }
    }

    fn dispatch_inner(&self, cmd: &str, args: Value) -> CoreResult<Value> {
        let ctx = self.kernel.context();

        match cmd {
            // =========================================================================
            // 00. Microkernel, WindowPoolManager, Shortcuts & Config IPC
            // =========================================================================
            "kernel_get_status" => {
                let active_windows = ctx.window_pool.list_active_windows();
                let shortcuts = ctx.shortcut_registry.list_all();
                let modules = self.kernel.registry().list_modules();
                let config = ctx.config.read().clone();
                let fg_window = self.window_pin.current_foreground_window().ok();
                let env_tools = vec![
                    ctx.env_probe.probe(ExternalToolKind::Python3, false),
                    ctx.env_probe.probe(ExternalToolKind::Osascript, false),
                    ctx.env_probe.probe(ExternalToolKind::Zsh, false),
                    ctx.env_probe.probe(ExternalToolKind::Node, false),
                    ctx.env_probe.probe(ExternalToolKind::Ffmpeg, false),
                ];

                Ok(json!({
                    "target_os": TargetOs::current(),
                    "active_window_count": active_windows.len(),
                    "active_windows": active_windows,
                    "modules": modules,
                    "shortcuts": shortcuts,
                    "config": config,
                    "clipboard_suppressed": ctx.event_bus.is_clipboard_suppressed(),
                    "paste_trigger_count": self.os_hal.paste_trigger_count(),
                    "foreground_external_window": fg_window,
                    "env_tools": env_tools,
                }))
            }

            "kernel_trigger_shortcut" => {
                if let Some(action_id) = args.get("action_id").and_then(Value::as_str) {
                    self.kernel.trigger_action(action_id)?;
                } else if let Some(accel) = args.get("accelerator").and_then(Value::as_str) {
                    self.kernel.trigger_accelerator(accel)?;
                } else {
                    return Err(app_core::CoreError::Config(
                        "Missing 'action_id' or 'accelerator'".to_string(),
                    ));
                }
                Ok(json!({
                    "active_window_count": ctx.window_pool.active_window_count(),
                    "active_windows": ctx.window_pool.list_active_windows(),
                }))
            }

            "kernel_open_dashboard" => {
                let route = args
                    .get("route")
                    .and_then(Value::as_str)
                    .unwrap_or("/overview");
                ctx.window_pool.open_window(WindowSpec::dashboard(route))?;
                Ok(json!({
                    "active_window_count": ctx.window_pool.active_window_count(),
                    "active_windows": ctx.window_pool.list_active_windows(),
                }))
            }

            "kernel_destroy_window" => {
                let label = args.get("label").and_then(Value::as_str).unwrap_or("");
                let destroyed = ctx.window_pool.destroy_window(label)?;
                Ok(json!({
                    "destroyed": destroyed,
                    "active_window_count": ctx.window_pool.active_window_count(),
                    "active_windows": ctx.window_pool.list_active_windows(),
                }))
            }

            "kernel_blur_window" => {
                let label = args.get("label").and_then(Value::as_str).unwrap_or("");
                let auto_destroyed = ctx.window_pool.on_window_blur(label)?;
                Ok(json!({
                    "auto_destroyed": auto_destroyed,
                    "active_window_count": ctx.window_pool.active_window_count(),
                    "active_windows": ctx.window_pool.list_active_windows(),
                }))
            }

            "kernel_update_clipboard_config" => {
                let max_items = args
                    .get("max_history_items")
                    .and_then(Value::as_u64)
                    .unwrap_or(100) as usize;
                let enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                self.kernel.update_config(|cfg| {
                    cfg.clipboard.max_history_items = max_items.clamp(1, 500);
                    cfg.clipboard.enabled = enabled;
                })?;
                Ok(serde_json::to_value(ctx.config.read().clone())?)
            }

            "kernel_shutdown" => {
                self.kernel.shutdown()?;
                Ok(json!({
                    "active_window_count": ctx.window_pool.active_window_count(),
                    "pinned_external_windows": self.window_pin.list_pinned_windows(),
                }))
            }

            // =========================================================================
            // 02. Clipboard Manager (`mod-clipboard`) IPC
            // =========================================================================
            "clipboard_capture_text" => {
                let text = args.get("text").and_then(Value::as_str).unwrap_or("");
                let source_app = args.get("source_app_name").and_then(Value::as_str);
                let record = self.clipboard.capture_text(ctx, text, source_app)?;
                Ok(serde_json::to_value(record)?)
            }

            "clipboard_list_items" => {
                let keyword = args.get("keyword").and_then(Value::as_str);
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize;
                let items = self.clipboard.list_items(ctx, keyword, limit)?;
                Ok(serde_json::to_value(items)?)
            }

            "clipboard_paste_item" => {
                let id = args.get("id").and_then(Value::as_i64).unwrap_or(0);
                let as_plain = args
                    .get("as_plain_text")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                // 屏蔽剪贴板自捕获，避免写回操作系统剪贴板时再次被后台监听器重复记录
                ctx.event_bus.set_clipboard_suppression(true, "quick_paste");
                let record = self.clipboard.paste_item(ctx, id, as_plain);
                if let Ok(ref rec) = record {
                    match rec.item_type {
                        mod_clipboard::ClipboardItemType::Text => {
                            if let Some(ref text) = rec.content_text {
                                write_text_to_system_clipboard(text)?;
                            }
                        }
                        mod_clipboard::ClipboardItemType::Image => {
                            if let Some(ref rel_path) = rec.blob_rel_path {
                                let full_path = ctx.app_data_dir.join(rel_path);
                                if full_path.exists() {
                                    write_image_to_system_clipboard(&full_path)?;
                                }
                            }
                        }
                    }
                }
                ctx.event_bus
                    .set_clipboard_suppression(false, "quick_paste_done");
                let record = record?;
                Ok(json!({
                    "pasted_record": record,
                    "paste_trigger_count": self.os_hal.paste_trigger_count(),
                    "active_window_count": ctx.window_pool.active_window_count(),
                }))
            }

            "clipboard_toggle_pin" => {
                let id = args.get("id").and_then(Value::as_i64).unwrap_or(0);
                let is_pinned = self.clipboard.toggle_pin(ctx, id)?;
                Ok(json!({ "id": id, "is_pinned": is_pinned }))
            }

            "clipboard_delete_item" => {
                let id = args.get("id").and_then(Value::as_i64).unwrap_or(0);
                let deleted = self.clipboard.delete_item(ctx, id)?;
                Ok(json!({ "id": id, "deleted": deleted }))
            }

            "clipboard_clear_unpinned" => {
                let deleted_count = self.clipboard.clear_unpinned(ctx)?;
                Ok(json!({ "deleted_count": deleted_count }))
            }

            "clipboard_set_suppression" => {
                let suppressed = args
                    .get("suppressed")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let reason = args
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("manual_ui_toggle");
                ctx.event_bus.set_clipboard_suppression(suppressed, reason);
                Ok(json!({
                    "clipboard_suppressed": ctx.event_bus.is_clipboard_suppressed()
                }))
            }

            // =========================================================================
            // 04. Desktop Pinned Sticky Notes (`mod-sticky-note`) IPC
            // =========================================================================
            "sticky_create_note" => {
                let title = args.get("title").and_then(Value::as_str);
                let content_md = args.get("content_md").and_then(Value::as_str);
                let theme_color = args.get("theme_color").and_then(Value::as_str);
                let note = self
                    .sticky_note
                    .create_note(ctx, title, content_md, theme_color)?;
                Ok(serde_json::to_value(note)?)
            }

            "sticky_list_notes" => {
                let only_open = args
                    .get("only_open")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let notes = self.sticky_note.list_notes(ctx, only_open)?;
                Ok(serde_json::to_value(notes)?)
            }

            "sticky_get_note" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or("");
                let note = self.sticky_note.get_note(ctx, id)?;
                Ok(serde_json::to_value(note)?)
            }

            "sticky_open_note" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or("");
                let note = self.sticky_note.open_note(ctx, id)?;
                Ok(serde_json::to_value(note)?)
            }

            "sticky_save_content" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or("");
                let title = args.get("title").and_then(Value::as_str);
                let content_md = args.get("content_md").and_then(Value::as_str).unwrap_or("");
                let updated = self.sticky_note.save_content(ctx, id, title, content_md)?;
                Ok(serde_json::to_value(updated)?)
            }

            "sticky_update_window_state" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or("");
                let current = self.sticky_note.get_note(ctx, id)?.ok_or_else(|| {
                    app_core::CoreError::Internal(anyhow::anyhow!("Note '{}' not found", id))
                })?;
                let update = StickyNoteWindowStateUpdate {
                    pos_x: args
                        .get("pos_x")
                        .and_then(Value::as_i64)
                        .map(|v| v as i32)
                        .or(current.pos_x),
                    pos_y: args
                        .get("pos_y")
                        .and_then(Value::as_i64)
                        .map(|v| v as i32)
                        .or(current.pos_y),
                    width: args
                        .get("width")
                        .and_then(Value::as_u64)
                        .map(|v| v as u32)
                        .unwrap_or(current.width),
                    height: args
                        .get("height")
                        .and_then(Value::as_u64)
                        .map(|v| v as u32)
                        .unwrap_or(current.height),
                    is_pinned: args
                        .get("is_pinned")
                        .and_then(Value::as_bool)
                        .unwrap_or(current.is_pinned),
                    opacity: args
                        .get("opacity")
                        .and_then(Value::as_f64)
                        .unwrap_or(current.opacity),
                };
                self.sticky_note.update_window_state(ctx, id, update)?;
                let refreshed = self.sticky_note.get_note(ctx, id)?;
                Ok(serde_json::to_value(refreshed)?)
            }

            "sticky_update_theme" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or("");
                let theme_color = args
                    .get("theme_color")
                    .and_then(Value::as_str)
                    .unwrap_or("amber");
                ctx.db.with_conn(|conn| {
                    conn.execute(
                        "UPDATE sticky_notes SET theme_color = ?1 WHERE id = ?2",
                        [theme_color, id],
                    )?;
                    Ok(())
                })?;
                let refreshed = self.sticky_note.get_note(ctx, id)?;
                Ok(serde_json::to_value(refreshed)?)
            }

            "sticky_close_note" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or("");
                self.sticky_note.close_note(ctx, id)?;
                Ok(json!({
                    "id": id,
                    "closed": true,
                    "active_window_count": ctx.window_pool.active_window_count()
                }))
            }

            "sticky_delete_note" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or("");
                let deleted = self.sticky_note.delete_note(ctx, id)?;
                Ok(json!({
                    "id": id,
                    "deleted": deleted,
                    "active_window_count": ctx.window_pool.active_window_count()
                }))
            }

            "sticky_export_markdown" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or("");
                let markdown = self.sticky_note.export_markdown(ctx, id)?;
                Ok(json!({ "id": id, "markdown": markdown }))
            }

            // =========================================================================
            // 09. DeskPins External Window Topmost (`mod-window-pin`) IPC
            // =========================================================================
            "winpin_sync_os_windows" => {
                let raw_windows = args
                    .get("windows")
                    .cloned()
                    .unwrap_or_else(|| Value::Array(Vec::new()));
                let parsed: Vec<ExternalWindowInfo> =
                    serde_json::from_value(raw_windows).unwrap_or_default();
                let fg_handle = args.get("foreground_handle").and_then(Value::as_u64);
                if !parsed.is_empty() {
                    self.os_hal
                        .mock()
                        .replace_external_windows(parsed, fg_handle);
                }
                let candidates = self.window_pin.list_candidate_windows()?;
                let fg = self.window_pin.current_foreground_window().ok();
                Ok(json!({
                    "candidates": candidates,
                    "foreground_handle": fg.map(|w| w.handle),
                    "pinned": self.window_pin.list_pinned_windows(),
                }))
            }

            "winpin_list_candidates" => {
                let candidates = self.window_pin.list_candidate_windows()?;
                let fg = self.window_pin.current_foreground_window().ok();
                Ok(json!({
                    "candidates": candidates,
                    "foreground_handle": fg.map(|w| w.handle),
                    "pinned": self.window_pin.list_pinned_windows(),
                }))
            }

            "winpin_list_all" => {
                let pinned = self.window_pin.list_pinned_windows();
                Ok(serde_json::to_value(pinned)?)
            }

            "winpin_set_foreground" => {
                let handle = args.get("handle").and_then(Value::as_u64).unwrap_or(1001);
                self.os_hal.mock().set_foreground_handle(handle);
                let fg = self.window_pin.current_foreground_window()?;
                Ok(serde_json::to_value(fg)?)
            }

            "winpin_toggle_foreground" => {
                let (info, is_pinned) = self.window_pin.toggle_foreground_window()?;
                if !is_pinned {
                    let badge_label = format!("pin-badge-{}", info.handle);
                    let _ = ctx.window_pool.destroy_window(&badge_label);
                }
                Ok(json!({
                    "window": info,
                    "is_pinned": is_pinned,
                    "pinned": self.window_pin.list_pinned_windows(),
                }))
            }

            "winpin_toggle_handle" => {
                let handle = args.get("handle").and_then(Value::as_u64).unwrap_or(0);
                let (info, is_pinned) = self.window_pin.toggle_handle(handle)?;
                if !is_pinned {
                    let badge_label = format!("pin-badge-{}", handle);
                    let _ = ctx.window_pool.destroy_window(&badge_label);
                }
                Ok(json!({
                    "window": info,
                    "is_pinned": is_pinned,
                    "pinned": self.window_pin.list_pinned_windows(),
                }))
            }

            "winpin_pick_at_cursor" => {
                let x = args.get("x").and_then(Value::as_i64).unwrap_or(0) as i32;
                let y = args.get("y").and_then(Value::as_i64).unwrap_or(0) as i32;
                let (info, is_pinned) = self.window_pin.pick_at_cursor(x, y)?;
                // 拾取完成后自动销毁全屏准星选择窗口
                let _ = ctx.window_pool.destroy_window("window-pin-picker");
                Ok(json!({
                    "window": info,
                    "is_pinned": is_pinned,
                    "pinned": self.window_pin.list_pinned_windows(),
                    "active_window_count": ctx.window_pool.active_window_count(),
                }))
            }

            "winpin_get_snapshot" => {
                let handle = args.get("handle").and_then(Value::as_u64).unwrap_or(0);
                let data_url = self
                    .os_hal
                    .capture_window_data_url(handle)
                    .map_err(|error| CoreError::Internal(anyhow::anyhow!(error)))?;
                Ok(json!({ "handle": handle, "data_url": data_url }))
            }

            "winpin_activate" => {
                let handle = args.get("handle").and_then(Value::as_u64).unwrap_or(0);
                self.os_hal
                    .activate_external_window(handle)
                    .map_err(|error| CoreError::Internal(anyhow::anyhow!(error)))?;
                Ok(json!({ "handle": handle, "activated": true }))
            }

            "winpin_unpin" => {
                let handle = args.get("handle").and_then(Value::as_u64).unwrap_or(0);
                let info = self.window_pin.unpin_window(handle)?;
                let badge_label = format!("pin-badge-{}", handle);
                let _ = ctx.window_pool.destroy_window(&badge_label);
                Ok(json!({
                    "window": info,
                    "pinned": self.window_pin.list_pinned_windows(),
                }))
            }

            "winpin_restore_all" => {
                let current_pinned = self.window_pin.list_pinned_windows();
                for item in current_pinned {
                    let badge_label = format!("pin-badge-{}", item.info.handle);
                    let _ = ctx.window_pool.destroy_window(&badge_label);
                }
                let restored = self.window_pin.restore_all_pinned_windows();
                Ok(json!({
                    "restored_count": restored,
                    "pinned": self.window_pin.list_pinned_windows(),
                }))
            }

            other => Err(app_core::CoreError::Config(format!(
                "Unknown IPC command: '{}'",
                other
            ))),
        }
    }
}

/// Starts the local HTTP + IPC bridge server serving `src_ui_dir` and `POST /api/ipc`.
pub fn serve_desktop_ui(
    host: Arc<DesktopHost>,
    listener: TcpListener,
    src_ui_dir: PathBuf,
) -> std::io::Result<()> {
    for stream_res in listener.incoming() {
        match stream_res {
            Ok(stream) => {
                let host_ref = Arc::clone(&host);
                let ui_dir = src_ui_dir.clone();
                std::thread::spawn(move || {
                    if let Err(err) = handle_http_connection(stream, &host_ref, &ui_dir) {
                        error!(error = %err, "HTTP/IPC connection error");
                    }
                });
            }
            Err(err) => {
                error!(error = %err, "TCP accept error");
            }
        }
    }
    Ok(())
}

fn handle_http_connection(
    mut stream: TcpStream,
    host: &DesktopHost,
    ui_dir: &Path,
) -> std::io::Result<()> {
    let mut buf = [0u8; 65536];
    let n = stream.read(&mut buf)?;
    if n == 0 {
        return Ok(());
    }

    let raw_req = String::from_utf8_lossy(&buf[..n]);
    let header_end = raw_req.find("\r\n\r\n").unwrap_or(raw_req.len());
    let header_part = &raw_req[..header_end];
    let mut lines = header_part.lines();
    let first_line = lines.next().unwrap_or("");
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let raw_path = parts.next().unwrap_or("/");
    let path = raw_path.split('?').next().unwrap_or("/");

    let mut content_length = 0usize;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse::<usize>().unwrap_or(0);
            }
        }
    }

    let body_start = if header_end + 4 <= n {
        header_end + 4
    } else {
        n
    };
    let mut body_bytes = buf[body_start..n].to_vec();
    while body_bytes.len() < content_length {
        let mut more = vec![0u8; content_length - body_bytes.len()];
        let read_more = stream.read(&mut more)?;
        if read_more == 0 {
            break;
        }
        body_bytes.extend_from_slice(&more[..read_more]);
    }

    if method == "OPTIONS" {
        return write_http_response(&mut stream, "200 OK", "text/plain", b"");
    }

    if method == "POST" && path == "/api/ipc" {
        let parsed: Result<IpcRequest, _> = serde_json::from_slice(&body_bytes);
        let resp = match parsed {
            Ok(req) => host.invoke_ipc(&req.cmd, req.args),
            Err(err) => IpcResponse::err(format!("Invalid IPC JSON request: {}", err)),
        };
        let json_bytes = serde_json::to_vec(&resp).unwrap_or_else(|_| b"{}".to_vec());
        return write_http_response(
            &mut stream,
            "200 OK",
            "application/json; charset=utf-8",
            &json_bytes,
        );
    }

    // Serve static files from `src-ui/`
    let rel_file = match path {
        "/" | "/index.html" => "index.html",
        "/styles.css" => "styles.css",
        "/app.js" => "app.js",
        _ => "index.html",
    };

    let file_path = ui_dir.join(rel_file);
    let content_type = match rel_file {
        "styles.css" => "text/css; charset=utf-8",
        "app.js" => "application/javascript; charset=utf-8",
        _ => "text/html; charset=utf-8",
    };

    match std::fs::read(&file_path) {
        Ok(bytes) => write_http_response(&mut stream, "200 OK", content_type, &bytes),
        Err(_) => write_http_response(
            &mut stream,
            "404 Not Found",
            "text/plain; charset=utf-8",
            b"404 Not Found",
        ),
    }
}

fn write_http_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let header = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: Content-Type\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        status,
        content_type,
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_desktop_host_ipc_full_workflow() {
        let temp_dir = tempfile::tempdir().unwrap();
        let host = DesktopHost::bootstrap(temp_dir.path(), true).unwrap();
        host.seed_demo_data_if_empty().unwrap();

        // 1. Verify Level 0 startup (0 active WebViews)
        let status = host.invoke_ipc("kernel_get_status", json!({}));
        assert!(status.ok);
        let data = status.data.unwrap();
        assert_eq!(data["active_window_count"], 0);
        assert_eq!(data["modules"].as_array().unwrap().len(), 3);

        // 2. Trigger Clipboard shortcut -> opens `clipboard-popup` (1 active WebView)
        let trig = host.invoke_ipc(
            "kernel_trigger_shortcut",
            json!({ "action_id": "clipboard:toggle_popup" }),
        );
        assert!(trig.ok);
        assert_eq!(trig.data.unwrap()["active_window_count"], 1);

        // 3. Capture new clipboard text and paste it -> automatically destroys `clipboard-popup`
        let cap = host.invoke_ipc(
            "clipboard_capture_text",
            json!({ "text": "Hello Desktop IPC", "source_app_name": "Code" }),
        );
        assert!(cap.ok);
        let item_id = cap.data.unwrap()["id"].as_i64().unwrap();

        let pasted = host.invoke_ipc(
            "clipboard_paste_item",
            json!({ "id": item_id, "as_plain_text": true }),
        );
        assert!(pasted.ok);
        let pasted_data = pasted.data.unwrap();
        assert_eq!(pasted_data["active_window_count"], 0);
        assert_eq!(pasted_data["paste_trigger_count"], 1);

        // 4. Create Sticky Note -> opens `sticky-note-{id}` PinnedResident WebView
        let note_resp = host.invoke_ipc(
            "sticky_create_note",
            json!({
                "title": "IPC 测试便笺",
                "content_md": "公式：$\\alpha + \\beta = \\gamma$",
                "theme_color": "emerald"
            }),
        );
        assert!(note_resp.ok);
        let note_id = note_resp.data.unwrap()["id"].as_str().unwrap().to_string();
        assert_eq!(host.kernel().context().window_pool.active_window_count(), 1);

        // Close sticky note -> back to 0 active WebViews
        let close_resp = host.invoke_ipc("sticky_close_note", json!({ "id": note_id }));
        assert!(close_resp.ok);
        assert_eq!(close_resp.data.unwrap()["active_window_count"], 0);

        // 5. Pin external window via crosshair coordinates (250, 200) and verify shutdown auto-restore
        let pick_resp = host.invoke_ipc("winpin_pick_at_cursor", json!({ "x": 250, "y": 200 }));
        assert!(pick_resp.ok);
        assert_eq!(
            pick_resp.data.unwrap()["pinned"].as_array().unwrap().len(),
            1
        );

        let shut_resp = host.invoke_ipc("kernel_shutdown", json!({}));
        assert!(shut_resp.ok);
        assert_eq!(
            shut_resp.data.unwrap()["pinned_external_windows"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
    }
}
