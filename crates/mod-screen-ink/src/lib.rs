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
//!      与透明墨迹 PNG 进行 Alpha 像素级叠加，支持保存为 PNG 文件与直接复制到系统剪贴板。
//! 4. **零后台常驻内存与整洁生命周期**：
//!    - 未启动时不创建任何 WebView；退出或模块停用时立即销毁全部窗口并释放资源。

pub mod composite;
pub mod config;

use anyhow::{anyhow, Context};
use app_core::{
    AppModule, CoreError, CoreResult, ModuleContext, ShortcutSpec, SystemEvent, WindowSpec,
};
use config::ScreenInkConfig;
use os_hal::{CapturedFrame, HalError, ScreenCapturer};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{borrow::Cow, fs, path::PathBuf, sync::Arc};
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
    pub width: u32,
    pub height: u32,
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

    /// 启动屏幕注释会话：创建全屏透明画布与独立置顶胶囊工具栏
    pub fn start_session(&self, ctx: &ModuleContext) -> CoreResult<ScreenInkSessionSnapshot> {
        let (screen_w, screen_h) = self.resolve_primary_screen_size()?;

        // 生成规范预设的双窗口
        let (canvas_spec, toolbar_spec) = WindowSpec::screen_ink_pair(screen_w, screen_h);

        // 打开全屏透明画布
        ctx.window_pool.open_window(canvas_spec)?;
        // 打开独立置顶工具栏
        ctx.window_pool.open_window(toolbar_spec)?;

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

    /// 退出屏幕注释会话：彻底销毁画布与工具栏窗口，重置状态
    pub fn exit_session(&self, ctx: &ModuleContext) -> CoreResult<()> {
        let _ = ctx.window_pool.destroy_window("screen-ink-canvas");
        let _ = ctx.window_pool.destroy_window("screen-ink-toolbar");

        {
            let mut guard = self.state.write();
            guard.is_active = false;
            guard.is_passthrough = false;
        }

        let snapshot = self.get_status();
        let _ = ctx.event_bus.publish(SystemEvent::CustomModuleEvent {
            source_module: "screen_ink".to_string(),
            topic: "screen-ink://mode-changed".to_string(),
            payload_json: json!(snapshot).to_string(),
        });
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

    /// 捕获屏幕底图并与墨迹 PNG 合成，按需保存至文件及复制到系统剪贴板
    pub fn composite_and_save(
        &self,
        ctx: &ModuleContext,
        ink_png_bytes: &[u8],
        copy_to_clipboard: bool,
        target_file_path: Option<String>,
        mock_bg_frame: Option<CapturedFrame>,
    ) -> CoreResult<SaveSnapshotResult> {
        // 1. 获取屏幕真实单帧底图（禁止循环录屏）
        let bg_frame = match mock_bg_frame {
            Some(frame) => frame,
            None => self
                .capturer
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
                })?,
        };

        // 2. 执行底图与透明墨迹像素 Alpha 叠加混合
        let (composited_png, width, height) = composite::composite_screen_with_ink(
            bg_frame.width,
            bg_frame.height,
            &bg_frame.rgba_bytes,
            ink_png_bytes,
        )?;

        // 3. 落盘保存文件
        let saved_path_string = {
            let path_to_write = if let Some(p) = target_file_path {
                PathBuf::from(p)
            } else {
                let snapshots_dir = ctx.app_data_dir.join("snapshots");
                fs::create_dir_all(&snapshots_dir)
                    .context("Failed to create snapshots directory")
                    .map_err(|e| CoreError::Internal(e))?;
                let now_str = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
                snapshots_dir.join(format!("screen_ink_{}.png", now_str))
            };

            if let Some(parent) = path_to_write.parent() {
                let _ = fs::create_dir_all(parent);
            }

            fs::write(&path_to_write, &composited_png)
                .context(format!("Failed to write snapshot to {:?}", path_to_write))
                .map_err(|e| CoreError::Internal(e))?;

            Some(path_to_write.to_string_lossy().to_string())
        };

        // 4. 复制到系统剪贴板（使用真实 arboard 原生系统剪贴板）
        let mut copied = false;
        if copy_to_clipboard {
            match self.copy_png_to_clipboard(&composited_png, width, height) {
                Ok(()) => {
                    copied = true;
                }
                Err(e) => {
                    warn!(error = %e, "Failed to copy composited snapshot to system clipboard");
                }
            }
        }

        Ok(SaveSnapshotResult {
            saved_path: saved_path_string,
            copied_to_clipboard: copied,
            width,
            height,
        })
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
        // 读取模块配置，确认配置结构有效性
        let _: ScreenInkConfig = ctx.module_settings(self.id())?;
        // 初始状态下不创建任何常驻窗口，遵循 <25MB 内存规范
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
        // 关闭或程序退出时，安全销毁可能残留的画布与工具栏窗口
        let _ = self.exit_session(ctx);
        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests/unit/lib.rs"]
mod tests;
