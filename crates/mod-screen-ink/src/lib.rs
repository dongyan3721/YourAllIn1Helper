//! 模块 03：桌面屏幕注释（gInk 风格）核心领域逻辑。
//!
//! 关键特性：
//! 1. **双窗口真正分离策略**：
//!    - `screen-ink-canvas`：全屏透明无边框置顶画布，负责压感墨迹、荧光笔、激光笔与几何图形渲染。
//!    - `screen-ink-toolbar`：独立置顶胶囊工具栏，始终保持鼠标可点击。
//! 2. **桌面穿透模式**：
//!    - 切换到操作桌面模式时，对全屏画布启用 `set_mouse_passthrough(true)`，
//!      保留所有透明墨迹的同时允许直接操作底层桌面与应用；工具栏不受穿透影响，可随时切回绘制。
//! 3. **底图捕获与墨迹合成**：
//!    - 通过注入的 `Arc<dyn ScreenCapturer>` 仅在保存时按需捕获单帧屏幕底图，
//!      截屏期间临时隐藏自身窗口避免双重叠墨或截入工具栏；独占创建文件避免覆盖；
//!      与透明墨迹 PNG 进行 Alpha 像素级叠加，写入系统剪贴板并如实反馈结果。
//! 4. **零后台常驻内存与可靠生命周期**：
//!    - 未启动时不创建任何 WebView；退出或模块停用时立即销毁全部窗口并释放资源；
//!      窗口创建失败具备原子回滚能力，销毁失败向调用方正确返回错误。

pub mod composite;
pub mod config;

use anyhow::{anyhow, Context};
use app_core::{
    AppModule, CoreError, CoreResult, ModuleContext, ShortcutSpec, SystemEvent, WindowPoolManager,
    WindowSpec,
};
use config::ScreenInkConfig;
use os_hal::{HalError, ScreenCapturer};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    borrow::Cow,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::Arc,
    thread,
    time::Duration,
};
use tracing::{info, warn};

/// 屏幕注释当前运行态会话快照
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenInkSessionSnapshot {
    pub is_active: bool,
    pub is_passthrough: bool,
    pub active_tool: String,
    pub color: String,
    pub stroke_width: f64,
}

/// 快照合成与保存/复制返回结果
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveSnapshotResult {
    pub saved_path: Option<String>,
    pub copied_to_clipboard: bool,
    pub clipboard_error: Option<String>,
    pub width: u32,
    pub height: u32,
}

/// RAII 守护：在截屏时临时隐藏画布与工具栏，确保异常或提前返回时 100% 恢复窗口可见性
struct CleanCaptureVisibilityGuard<'a> {
    window_pool: &'a WindowPoolManager,
    labels: &'static [&'static str],
}

impl<'a> CleanCaptureVisibilityGuard<'a> {
    fn new(window_pool: &'a WindowPoolManager, labels: &'static [&'static str]) -> Self {
        for label in labels {
            let _ = window_pool.set_window_visible(label, false);
        }
        Self {
            window_pool,
            labels,
        }
    }
}

impl<'a> Drop for CleanCaptureVisibilityGuard<'a> {
    fn drop(&mut self) {
        for label in self.labels {
            let _ = self.window_pool.set_window_visible(label, true);
        }
    }
}

/// 屏幕注释领域模型与生命周期控制器
pub struct ScreenInkModule {
    capturer: Arc<dyn ScreenCapturer>,
    state: RwLock<ScreenInkSessionSnapshot>,
}

impl ScreenInkModule {
    pub fn new(capturer: Arc<dyn ScreenCapturer>) -> Self {
        Self {
            capturer,
            state: RwLock::new(ScreenInkSessionSnapshot {
                is_active: false,
                is_passthrough: false,
                active_tool: "pen".to_string(),
                color: "#ef4444".to_string(),
                stroke_width: 4.0,
            }),
        }
    }

    /// 获取当前会话状态快照
    pub fn get_status(&self) -> ScreenInkSessionSnapshot {
        self.state.read().clone()
    }

    /// 应用或更新配置（并自动清洗非法字段）
    pub fn apply_config(&self, mut cfg: ScreenInkConfig) {
        cfg.sanitize();
        let mut guard = self.state.write();
        guard.active_tool = cfg.default_tool;
        guard.color = cfg.default_color;
        guard.stroke_width = cfg.default_width;
    }

    /// 启动屏幕注释会话：创建全屏透明画布与独立置顶胶囊工具栏
    /// 若第二窗口创建失败，必须原子回滚销毁第一窗口，保持状态与实际资源一致
    pub fn start_session(&self, ctx: &ModuleContext) -> CoreResult<ScreenInkSessionSnapshot> {
        let (screen_w, screen_h) = self.resolve_primary_screen_size()?;
        let (canvas_spec, toolbar_spec) = WindowSpec::screen_ink_pair(screen_w, screen_h);

        // 1. 打开全屏透明画布
        ctx.window_pool.open_window(canvas_spec)?;

        // 2. 打开独立置顶工具栏（失败时立即回滚销毁全屏画布）
        if let Err(err) = ctx.window_pool.open_window(toolbar_spec) {
            let _ = ctx.window_pool.destroy_window("screen-ink-canvas");
            return Err(err);
        }

        // 3. 关键：若此前曾处于穿透模式，重复/重新启动必须重置原生画布为可接收交互，确保原生与状态一致
        let _ = ctx
            .window_pool
            .set_mouse_passthrough("screen-ink-canvas", false);

        {
            let mut guard = self.state.write();
            guard.is_active = true;
            guard.is_passthrough = false;
        }

        let snapshot = self.get_status();
        let _ = ctx.event_bus.publish(SystemEvent::CustomModuleEvent {
            source_module: "screen_ink".to_string(),
            topic: "screen-ink://mode-changed".to_string(),
            payload_json: json!(snapshot).to_string(),
        });
        info!(screen_w, screen_h, "Started screen ink dual-window session");
        Ok(snapshot)
    }

    /// 退出屏幕注释会话：彻底销毁画布与工具栏窗口，错误不被吞没并保持状态真实
    pub fn exit_session(&self, ctx: &ModuleContext) -> CoreResult<()> {
        let mut first_err: Option<CoreError> = None;

        if let Err(e) = ctx.window_pool.destroy_window("screen-ink-canvas") {
            first_err = Some(e);
        }
        if let Err(e) = ctx.window_pool.destroy_window("screen-ink-toolbar") {
            if first_err.is_none() {
                first_err = Some(e);
            }
        }

        let canvas_still_open = ctx
            .window_pool
            .get_active_window("screen-ink-canvas")
            .is_some();
        let toolbar_still_open = ctx
            .window_pool
            .get_active_window("screen-ink-toolbar")
            .is_some();
        let any_open = canvas_still_open || toolbar_still_open;

        {
            let mut guard = self.state.write();
            guard.is_active = any_open;
            if !any_open {
                guard.is_passthrough = false;
            }
        }

        let snapshot = self.get_status();
        let _ = ctx.event_bus.publish(SystemEvent::CustomModuleEvent {
            source_module: "screen_ink".to_string(),
            topic: "screen-ink://mode-changed".to_string(),
            payload_json: json!(snapshot).to_string(),
        });

        if let Some(err) = first_err {
            return Err(err);
        }

        info!("Exited screen ink session and destroyed both windows");
        Ok(())
    }

    /// 切换画布鼠标穿透状态（操作桌面模式 vs 绘制墨迹模式）
    /// 关键：仅让透明画布穿透点击，独立工具栏始终保持响应
    pub fn set_passthrough(
        &self,
        ctx: &ModuleContext,
        passthrough: bool,
    ) -> CoreResult<ScreenInkSessionSnapshot> {
        {
            let guard = self.state.read();
            if !guard.is_active {
                return Err(CoreError::Internal(anyhow!(
                    "Screen ink session is not currently active"
                )));
            }
        }

        // 调用窗口池的穿透设置：仅作用于画布窗口
        ctx.window_pool
            .set_mouse_passthrough("screen-ink-canvas", passthrough)?;

        {
            let mut guard = self.state.write();
            guard.is_passthrough = passthrough;
        }

        let snapshot = self.get_status();
        let _ = ctx.event_bus.publish(SystemEvent::CustomModuleEvent {
            source_module: "screen_ink".to_string(),
            topic: "screen-ink://mode-changed".to_string(),
            payload_json: json!(snapshot).to_string(),
        });
        info!(passthrough, "Toggled screen ink canvas mouse passthrough");
        Ok(snapshot)
    }

    /// 清空透明画布上的所有墨迹
    pub fn clear_canvas(&self, ctx: &ModuleContext) -> CoreResult<()> {
        let _ = ctx.event_bus.publish(SystemEvent::CustomModuleEvent {
            source_module: "screen_ink".to_string(),
            topic: "screen-ink://clear-canvas".to_string(),
            payload_json: "{}".to_string(),
        });
        Ok(())
    }

    /// 捕获屏幕真实单帧底图并与墨迹 PNG 合成
    /// 1. 临时隐藏自身窗口（避免双重叠墨与将工具栏截入）
    /// 2. 仅通过注入的 ScreenCapturer 抓取单帧，生产环境严禁假冒伪成功
    /// 3. 保存文件采用独占创建（create_new）防止覆盖已有文件
    /// 4. 剪贴板复制失败准确记录并反馈
    pub fn composite_and_save(
        &self,
        ctx: &ModuleContext,
        ink_png_bytes: &[u8],
        copy_to_clipboard: bool,
        target_file_path: Option<String>,
    ) -> CoreResult<SaveSnapshotResult> {
        // 1. 干净底图捕获：在抓屏期间临时隐藏自身窗口，避免双重叠墨以及工具栏截入快照中
        let bg_frame = {
            // RAII 守护确保无论成功还是失败，画布和工具栏都会恢复显示
            let _guard = CleanCaptureVisibilityGuard::new(
                &ctx.window_pool,
                &["screen-ink-canvas", "screen-ink-toolbar"],
            );

            // 让系统显示合成器有短暂时间（40ms）完成帧刷新
            thread::sleep(Duration::from_millis(40));

            self.capturer
                .capture_monitor(None)
                .map_err(|err| match err {
                    HalError::PermissionDenied {
                        capability,
                        guidance,
                    } => CoreError::Internal(anyhow!(
                        "屏幕截取权限不足 [{}]: {}",
                        capability,
                        guidance
                    )),
                    HalError::Unsupported(cap) => {
                        CoreError::Internal(anyhow!("当前系统不支持能力 '{}'", cap))
                    }
                    HalError::NotFound(msg) => {
                        CoreError::Internal(anyhow!("未发现可用显示器: {}", msg))
                    }
                    HalError::NativeCallFailed(msg) => {
                        CoreError::Internal(anyhow!("调用系统屏幕捕获 API 失败: {}", msg))
                    }
                })?
        };

        // 2. 执行底图与透明墨迹像素 Alpha 叠加混合
        let (composited_png, width, height) = composite::composite_screen_with_ink(
            bg_frame.width,
            bg_frame.height,
            &bg_frame.rgba_bytes,
            ink_png_bytes,
        )?;

        // 3. 落盘保存文件：独占创建（create_new），不覆盖已有文件
        let cfg: ScreenInkConfig = ctx.module_settings(self.id()).unwrap_or_default();
        let saved_path_string =
            self.save_png_exclusive(ctx, &cfg, &composited_png, target_file_path)?;

        // 4. 复制到系统剪贴板（使用真实 arboard 原生系统剪贴板）
        let mut copied = false;
        let mut clip_err = None;
        if copy_to_clipboard {
            match self.copy_png_to_clipboard(&composited_png, width, height) {
                Ok(()) => {
                    copied = true;
                }
                Err(e) => {
                    warn!(error = %e, "Failed to copy composited snapshot to system clipboard");
                    clip_err = Some(e.to_string());
                }
            }
        }

        Ok(SaveSnapshotResult {
            saved_path: Some(saved_path_string),
            copied_to_clipboard: copied,
            clipboard_error: clip_err,
            width,
            height,
        })
    }

    /// 独占创建文件保存 PNG，避免同秒或同名覆盖
    fn save_png_exclusive(
        &self,
        ctx: &ModuleContext,
        cfg: &ScreenInkConfig,
        png_bytes: &[u8],
        target_file_path: Option<String>,
    ) -> CoreResult<String> {
        let base_path = if let Some(p) = target_file_path {
            PathBuf::from(p)
        } else {
            let base_dir = if let Some(ref custom_dir) = cfg.save_directory {
                PathBuf::from(custom_dir)
            } else {
                ctx.app_data_dir.join("snapshots")
            };
            let now_str = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
            base_dir.join(format!("screen_ink_{}.png", now_str))
        };

        if let Some(parent) = base_path.parent() {
            fs::create_dir_all(parent)
                .context("Failed to create snapshot parent directory")
                .map_err(CoreError::from)?;
        }

        // 尝试以独占方式创建文件；若已存在则追加序号重试
        let mut attempt_path = base_path.clone();
        let mut counter = 1;
        let file = loop {
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&attempt_path)
            {
                Ok(f) => break f,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let file_stem = base_path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("screen_ink");
                    let ext = base_path
                        .extension()
                        .and_then(|s| s.to_str())
                        .unwrap_or("png");
                    if let Some(parent) = base_path.parent() {
                        attempt_path = parent.join(format!("{}_{}.{}", file_stem, counter, ext));
                    } else {
                        attempt_path = PathBuf::from(format!("{}_{}.{}", file_stem, counter, ext));
                    }
                    counter += 1;
                    if counter > 100 {
                        return Err(CoreError::Internal(anyhow!(
                            "Failed to allocate a unique snapshot file name after 100 attempts"
                        )));
                    }
                }
                Err(e) => {
                    return Err(CoreError::Internal(anyhow!(
                        "Failed to create snapshot file at {:?}: {}",
                        attempt_path,
                        e
                    )));
                }
            }
        };

        let mut writer = std::io::BufWriter::new(file);
        writer
            .write_all(png_bytes)
            .context("Failed to write snapshot image data")
            .map_err(CoreError::from)?;
        writer.flush().map_err(CoreError::from)?;

        Ok(attempt_path.to_string_lossy().to_string())
    }

    /// 将合并后的 PNG 图像解析为 RGBA 格式并写入系统剪贴板
    fn copy_png_to_clipboard(
        &self,
        png_bytes: &[u8],
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let dynamic_img = image::load_from_memory(png_bytes)?;
        let rgba = dynamic_img.to_rgba8();
        let mut clipboard = arboard::Clipboard::new()?;
        clipboard.set_image(arboard::ImageData {
            width: width as usize,
            height: height as usize,
            bytes: Cow::Borrowed(&rgba.into_raw()),
        })?;
        Ok(())
    }

    /// 解析主屏幕逻辑尺寸，兼容 HiDPI 缩放与多屏环境
    fn resolve_primary_screen_size(&self) -> CoreResult<(f64, f64)> {
        match self.capturer.list_monitors() {
            Ok(monitors) if !monitors.is_empty() => {
                let primary = monitors
                    .iter()
                    .find(|m| m.is_primary)
                    .unwrap_or(&monitors[0]);
                let scale = if primary.scale_factor > 0.1 {
                    primary.scale_factor
                } else {
                    1.0
                };
                let logical_w = primary.width as f64 / scale;
                let logical_h = primary.height as f64 / scale;
                Ok((logical_w.max(640.0), logical_h.max(480.0)))
            }
            _ => Ok((1920.0, 1080.0)),
        }
    }
}

impl AppModule for ScreenInkModule {
    fn id(&self) -> &'static str {
        "screen_ink"
    }

    fn name(&self) -> &'static str {
        "桌面屏幕注释"
    }

    fn default_shortcuts(&self) -> Vec<ShortcutSpec> {
        vec![
            ShortcutSpec::new(
                self.id(),
                "screen_ink:toggle_session",
                "PrimaryMod+Shift+D",
                "进入/退出屏幕注释",
            ),
            ShortcutSpec::new(
                self.id(),
                "screen_ink:toggle_passthrough",
                "PrimaryMod+Shift+E",
                "切换画笔/鼠标穿透",
            ),
        ]
    }

    fn on_init(&self, ctx: &ModuleContext) -> CoreResult<()> {
        // 读取并校验模块配置，应用至初始工具/颜色/线宽
        let cfg: ScreenInkConfig = ctx.module_settings(self.id()).unwrap_or_default();
        self.apply_config(cfg);

        let mut guard = self.state.write();
        guard.is_active = false;
        guard.is_passthrough = false;
        Ok(())
    }

    fn on_shortcut(&self, action_id: &str, ctx: &ModuleContext) -> CoreResult<()> {
        match action_id {
            "screen_ink:toggle_session" => {
                let is_active = self.state.read().is_active;
                if is_active {
                    self.exit_session(ctx)?;
                } else {
                    self.start_session(ctx)?;
                }
            }
            "screen_ink:toggle_passthrough" => {
                let (is_active, is_passthrough) = {
                    let guard = self.state.read();
                    (guard.is_active, guard.is_passthrough)
                };
                if is_active {
                    self.set_passthrough(ctx, !is_passthrough)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn on_shutdown(&self, ctx: &ModuleContext) -> CoreResult<()> {
        // 关闭或程序退出时，安全销毁可能残留的画布与工具栏窗口，并向调用者正确报告错误
        self.exit_session(ctx)
    }
}

#[cfg(test)]
#[path = "../tests/unit/lib.rs"]
mod tests;
