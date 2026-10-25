//! 模块 01：增强截图与贴图（M1：截图 → 编辑 → 保存/复制 → 独立贴图闭环）。
//!
//! 流程：
//! 1. 快捷键或主面板触发后先做屏幕录制权限预检，再经注入的 `ScreenCapturer` 抓取真实冻结帧；
//!    冻结帧完成后才创建（隐藏的）`capture-overlay`，前端绘制完成后再显示，桌面不会先露出遮罩。
//! 2. 窗口级智能区域来自注入的 `RegionDetector`（L1），层级求解复用 `app_core` 的算法；
//!    控件级（L2）与视觉候选（L3）在 M1 如实报告为不可用。
//! 3. 前端只把选区坐标与标注层 PNG 交回后端；裁剪、合成、编码、落盘、复制都在后端完成，
//!    冻结帧原始像素不回传。
//! 4. 贴图 `pin-img-{id}` 只在窗口存在时保留 PNG；关闭、停用、退出都销毁窗口并释放内存。
//!
//! 本模块不依赖 `NativeOsHal`、Tauri 或任何平台 API；系统剪贴板经 [`ScreenshotClipboard`] 注入。

pub mod clipboard;
pub mod config;
pub mod error;
pub mod export;
pub mod geometry;
pub mod pins;
pub mod session;

use app_core::{
    AppModule, CoreResult, ModuleContext, SelectionBox, ShortcutSpec, WindowPoolManager, WindowSpec,
};
use clipboard::ScreenshotClipboard;
use config::ScreenshotSettings;
use error::{internal, ScreenshotFailure};
use export::ExportAction;
use os_hal::{DisplayMonitorInfo, HalError, RegionDetector, ScreenCapturer};
use parking_lot::{Mutex, RwLock};
use pins::{is_valid_pin_id, pin_label, PinRecord, PinStore, PinSummary};
use serde::{Deserialize, Serialize};
use session::{
    CapabilityState, CaptureSession, OverlaySettings, RegionCapabilities, SessionInfo,
    SharedSession,
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};
use tracing::{info, warn};

/// 稳定模块 ID（与十槽位目录一致）。
pub const MODULE_ID: &str = "screenshot";
/// 区域截图快捷键动作。
pub const ACTION_CAPTURE: &str = "screenshot:capture";
/// 截图遮罩窗口标签。
pub const OVERLAY_LABEL: &str = "capture-overlay";
const DASHBOARD_LABEL: &str = "dashboard";
/// 隐藏主面板后等待窗口合成器刷新的时间。
const HIDE_SETTLE_MS: u64 = 60;
/// 停用/退出时等待进行中的截图事务的最长时间。
const SHUTDOWN_GATE_WAIT: Duration = Duration::from_secs(3);

/// 一次导出的结果；保存与复制的成败分别如实记录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportOutcome {
    pub action: ExportAction,
    pub width: u32,
    pub height: u32,
    pub saved_path: Option<String>,
    pub copied: bool,
    pub clipboard_error: Option<String>,
    pub pin: Option<PinSummary>,
    pub at: i64,
}

/// 屏幕录制权限的预检结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionStatus {
    pub granted: bool,
    pub message: Option<String>,
    pub guidance: Option<String>,
}

/// 当前会话摘要。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionBrief {
    pub session_id: String,
    pub monitor_name: String,
    pub overlay_visible: bool,
    pub created_at: i64,
}

/// 快捷键实际注册状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShortcutBrief {
    pub action_id: String,
    pub registered: bool,
    pub display_label: Option<String>,
    pub logical_hotkey: Option<String>,
}

/// `screenshot_get_status` 返回值。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenshotStatus {
    pub session: Option<SessionBrief>,
    pub pins: Vec<PinSummary>,
    pub last_export: Option<ExportOutcome>,
    pub last_error: Option<ScreenshotFailure>,
    pub permission: PermissionStatus,
    pub shortcut: ShortcutBrief,
    pub save_directory: String,
    pub settings: ScreenshotSettings,
}

/// 截图模块。
pub struct ScreenshotModule {
    capturer: Arc<dyn ScreenCapturer>,
    regions: Arc<dyn RegionDetector>,
    /// 宿主可在启动后替换为真实系统剪贴板；无桌面环境保持构造时注入的实现。
    clipboard: RwLock<Arc<dyn ScreenshotClipboard>>,
    /// 串行化会话的开始/导出/取消；开始截图只 `try_lock`，主线程快捷键永不阻塞等待。
    gate: Mutex<()>,
    session: RwLock<Option<SharedSession>>,
    overlay_visible: AtomicBool,
    pins: PinStore,
    last_export: RwLock<Option<ExportOutcome>>,
    last_error: RwLock<Option<ScreenshotFailure>>,
}

/// 抓帧期间临时隐藏主面板，作用域结束（含提前返回）时恢复显示。
struct HiddenWindowGuard<'a> {
    pool: &'a WindowPoolManager,
    label: Option<&'static str>,
}

impl<'a> HiddenWindowGuard<'a> {
    fn hide_if_open(pool: &'a WindowPoolManager, label: &'static str) -> CoreResult<Self> {
        if pool.get_active_window(label).is_none() {
            return Ok(Self { pool, label: None });
        }
        pool.set_window_visible(label, false)?;
        thread::sleep(Duration::from_millis(HIDE_SETTLE_MS));
        Ok(Self {
            pool,
            label: Some(label),
        })
    }
}

impl Drop for HiddenWindowGuard<'_> {
    fn drop(&mut self) {
        if let Some(label) = self.label.take() {
            if let Err(error) = self.pool.set_window_visible(label, true) {
                warn!(%error, label, "Failed to restore window hidden for screenshot");
            }
        }
    }
}

impl ScreenshotModule {
    pub fn new(
        capturer: Arc<dyn ScreenCapturer>,
        regions: Arc<dyn RegionDetector>,
        clipboard: Arc<dyn ScreenshotClipboard>,
    ) -> Self {
        Self {
            capturer,
            regions,
            clipboard: RwLock::new(clipboard),
            gate: Mutex::new(()),
            session: RwLock::new(None),
            overlay_visible: AtomicBool::new(false),
            pins: PinStore::default(),
            last_export: RwLock::new(None),
            last_error: RwLock::new(None),
        }
    }

    /// 替换剪贴板端口（生产宿主在 Tauri 启动后注入系统剪贴板）。
    pub fn set_clipboard(&self, clipboard: Arc<dyn ScreenshotClipboard>) {
        *self.clipboard.write() = clipboard;
    }

    fn clipboard(&self) -> Arc<dyn ScreenshotClipboard> {
        self.clipboard.read().clone()
    }

    // ───────────────────────────── 配置 ─────────────────────────────

    /// 读取模块配置；JSON 类型错误时退回缺省值并记录告警，不阻断截图。
    pub fn settings(&self, ctx: &ModuleContext) -> ScreenshotSettings {
        let mut settings: ScreenshotSettings =
            ctx.module_settings(MODULE_ID).unwrap_or_else(|error| {
                warn!(%error, "Invalid screenshot settings, falling back to defaults");
                ScreenshotSettings::default()
            });
        settings.sanitize();
        settings
    }

    /// 实际保存目录。
    pub fn save_directory(&self, ctx: &ModuleContext, settings: &ScreenshotSettings) -> PathBuf {
        settings
            .save_directory
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| ctx.app_data_dir.join("screenshots"))
    }

    fn overlay_settings(
        &self,
        ctx: &ModuleContext,
        settings: &ScreenshotSettings,
    ) -> OverlaySettings {
        let legacy = ctx.config.read().screenshot.clone();
        OverlaySettings {
            manual_selection: legacy.enable_manual_box_selection,
            smart_regions: legacy.enable_smart_region_detection,
            wheel_expand: legacy.smart_region_wheel_expand,
            magnifier: legacy.show_magnifier,
            default_color: settings.default_color.clone(),
            default_stroke_width: settings.default_stroke_width,
        }
    }

    // ───────────────────────────── 会话 ─────────────────────────────

    /// 开始区域截图。`hide_dashboard` 为 true 时（主面板按钮触发）抓帧期间隐藏主面板。
    ///
    /// 已有会话时以新冻结帧为准：先结束旧会话再重新抓取。
    pub fn start_capture(
        &self,
        ctx: &ModuleContext,
        monitor_id: Option<u32>,
        hide_dashboard: bool,
    ) -> CoreResult<SessionInfo> {
        let _gate = self
            .gate
            .try_lock()
            .ok_or_else(|| internal("上一次截图仍在处理中，请稍后再试"))?;
        if let Err(error) = self.finish_session_locked(ctx) {
            warn!(%error, "Failed to fully close previous capture overlay");
        }
        match self.begin_session(ctx, monitor_id, hide_dashboard) {
            Ok(info) => {
                *self.last_error.write() = None;
                Ok(info)
            }
            Err(failure) => {
                let error = failure.to_core_error();
                *self.last_error.write() = Some(failure);
                Err(error)
            }
        }
    }

    fn begin_session(
        &self,
        ctx: &ModuleContext,
        monitor_id: Option<u32>,
        hide_dashboard: bool,
    ) -> Result<SessionInfo, ScreenshotFailure> {
        // 1. 权限预检：缺权限时直接报告，不抓取「仅壁纸」的假画面。
        self.capturer
            .check_capture_permission()
            .map_err(|e| ScreenshotFailure::from_hal(&e))?;

        // 2. 选择显示器：指定 ID → 主屏 → 第一块。
        let monitors = self
            .capturer
            .list_monitors()
            .map_err(|e| ScreenshotFailure::from_hal(&e))?;
        let info = pick_monitor(&monitors, monitor_id).ok_or_else(|| {
            ScreenshotFailure::from_hal(&HalError::NotFound(match monitor_id {
                Some(id) => format!("显示器 {}", id),
                None => "没有可用显示器".to_string(),
            }))
        })?;

        // 3. 冻结帧先于遮罩：此时遮罩尚未创建，帧中只有用户真实桌面。
        let frame = {
            let _hidden = if hide_dashboard {
                Some(
                    HiddenWindowGuard::hide_if_open(&ctx.window_pool, DASHBOARD_LABEL)
                        .map_err(|e| ScreenshotFailure::internal(e.to_string()))?,
                )
            } else {
                None
            };
            self.capturer
                .capture_monitor(Some(info.id))
                .map_err(|e| ScreenshotFailure::from_hal(&e))?
        };
        let expected = frame.width as usize * frame.height as usize * 4;
        if frame.rgba_bytes.len() != expected {
            return Err(ScreenshotFailure::internal(format!(
                "系统返回的冻结帧像素长度异常：期望 {}，实际 {}",
                expected,
                frame.rgba_bytes.len()
            )));
        }
        let monitor = geometry::MonitorGeometry::new(&info, frame.width, frame.height)
            .map_err(|e| ScreenshotFailure::internal(e.to_string()))?;

        // 4. 窗口级智能区域：探测失败只降级为手动选区，并如实记录原因。
        let settings = self.settings(ctx);
        let overlay_settings = self.overlay_settings(ctx, &settings);
        let (detected, window_state) = if overlay_settings.smart_regions {
            match self.regions.detect_hierarchical_regions() {
                Ok(regions) => (regions, CapabilityState::Available),
                Err(error) => (
                    Vec::new(),
                    CapabilityState::Unavailable {
                        reason: error.to_string(),
                    },
                ),
            }
        } else {
            (Vec::new(), CapabilityState::Disabled)
        };
        let session = Arc::new(CaptureSession::new(
            monitor,
            frame,
            detected,
            RegionCapabilities::m1(window_state),
        ));

        // 5. 记录会话后再创建隐藏遮罩；创建失败时丢弃会话，冻结帧随之释放。
        *self.session.write() = Some(session.clone());
        self.overlay_visible.store(false, Ordering::SeqCst);
        let rect = session.monitor.overlay_logical_rect();
        if let Err(error) = ctx.window_pool.open_window(WindowSpec::capture_overlay(
            rect.x,
            rect.y,
            rect.width,
            rect.height,
        )) {
            *self.session.write() = None;
            let _ = ctx.window_pool.destroy_window(OVERLAY_LABEL);
            return Err(ScreenshotFailure::internal(format!(
                "创建截图遮罩窗口失败：{}",
                error
            )));
        }
        info!(
            session = %session.id,
            frame_w = session.frame.width,
            frame_h = session.frame.height,
            windows = session.windows.len(),
            "Screenshot frozen frame captured and overlay created"
        );
        Ok(session.describe(overlay_settings))
    }

    fn current_session(&self, session_id: &str) -> CoreResult<SharedSession> {
        self.session
            .read()
            .as_ref()
            .filter(|s| s.id == session_id)
            .cloned()
            .ok_or_else(|| internal("截图会话已结束或不存在"))
    }

    /// 当前是否有截图会话（供宿主和测试检查冻结帧是否已释放）。
    pub fn has_active_session(&self) -> bool {
        self.session.read().is_some()
    }

    /// 前端初始化所需的会话描述。
    pub fn session_info(
        &self,
        ctx: &ModuleContext,
        session_id: Option<&str>,
    ) -> CoreResult<SessionInfo> {
        let session = match session_id {
            Some(id) => self.current_session(id)?,
            None => self
                .session
                .read()
                .clone()
                .ok_or_else(|| internal("当前没有进行中的截图"))?,
        };
        let settings = self.settings(ctx);
        Ok(session.describe(self.overlay_settings(ctx, &settings)))
    }

    /// 冻结帧原始 RGBA 字节（经二进制 IPC 返回前端，复制一次，不做 JSON/base64 编码）。
    pub fn frame_bytes(&self, session_id: &str) -> CoreResult<Vec<u8>> {
        Ok(self.current_session(session_id)?.frame.rgba_bytes.clone())
    }

    /// 光标处的智能区域（帧像素）。
    pub fn resolve_region(
        &self,
        session_id: &str,
        x: i32,
        y: i32,
        level_steps: usize,
    ) -> CoreResult<Option<app_core::DetectedRegionRect>> {
        Ok(self
            .current_session(session_id)?
            .resolve_region(x, y, level_steps.min(8)))
    }

    /// 前端已绘制冻结帧：显示遮罩窗口（仅首次）。
    pub fn overlay_ready(&self, ctx: &ModuleContext, session_id: &str) -> CoreResult<()> {
        self.current_session(session_id)?;
        if !self.overlay_visible.swap(true, Ordering::SeqCst) {
            if let Err(error) = ctx.window_pool.set_window_visible(OVERLAY_LABEL, true) {
                self.overlay_visible.store(false, Ordering::SeqCst);
                return Err(error);
            }
        }
        Ok(())
    }

    /// 取消截图：释放冻结帧并销毁遮罩。会话 ID 过期时返回 `false` 而不影响新会话。
    pub fn cancel_capture(
        &self,
        ctx: &ModuleContext,
        session_id: Option<&str>,
    ) -> CoreResult<bool> {
        let _gate = self.gate.lock();
        if let Some(id) = session_id {
            let matches = self.session.read().as_ref().is_some_and(|s| s.id == id);
            if !matches {
                return Ok(false);
            }
        }
        self.finish_session_locked(ctx)?;
        Ok(true)
    }

    /// 结束会话：先丢弃冻结帧，再销毁遮罩；销毁失败也不会继续持有帧。
    fn finish_session_locked(&self, ctx: &ModuleContext) -> CoreResult<()> {
        let had_session = self.session.write().take().is_some();
        self.overlay_visible.store(false, Ordering::SeqCst);
        if had_session || ctx.window_pool.get_active_window(OVERLAY_LABEL).is_some() {
            ctx.window_pool.destroy_window(OVERLAY_LABEL)?;
        }
        Ok(())
    }

    // ───────────────────────────── 导出 ─────────────────────────────

    /// 按选区导出：复制 / 保存 / 贴图。成功后结束会话；失败时重新显示遮罩以便重试。
    pub fn export(
        &self,
        ctx: &ModuleContext,
        session_id: &str,
        selection: SelectionBox,
        overlay_png: Option<&[u8]>,
        action: ExportAction,
    ) -> CoreResult<ExportOutcome> {
        let _gate = self.gate.lock();
        let session = self.current_session(session_id)?;
        let selection =
            geometry::normalize_crop(&selection, session.frame.width, session.frame.height)?;
        let image = export::render_selection(&session.frame, &selection, overlay_png)?;
        let settings = self.settings(ctx);

        // 编码大图期间先隐藏遮罩，用户立即回到桌面。
        let _ = ctx.window_pool.set_window_visible(OVERLAY_LABEL, false);
        let result = self.perform_export(ctx, &session, &selection, image, action, &settings);
        match result {
            Ok(outcome) => {
                if let Err(error) = self.finish_session_locked(ctx) {
                    warn!(%error, "Export succeeded but overlay destroy failed");
                }
                *self.last_export.write() = Some(outcome.clone());
                *self.last_error.write() = None;
                Ok(outcome)
            }
            Err(error) => {
                *self.last_error.write() = Some(ScreenshotFailure::internal(error.to_string()));
                if self.overlay_visible.load(Ordering::SeqCst) {
                    let _ = ctx.window_pool.set_window_visible(OVERLAY_LABEL, true);
                }
                Err(error)
            }
        }
    }

    fn perform_export(
        &self,
        ctx: &ModuleContext,
        session: &CaptureSession,
        selection: &SelectionBox,
        image: image::RgbaImage,
        action: ExportAction,
        settings: &ScreenshotSettings,
    ) -> CoreResult<ExportOutcome> {
        let mut outcome = ExportOutcome {
            action,
            width: image.width(),
            height: image.height(),
            saved_path: None,
            copied: false,
            clipboard_error: None,
            pin: None,
            at: chrono::Utc::now().timestamp_millis(),
        };
        match action {
            ExportAction::Copy => {
                // 仅复制时剪贴板失败即整体失败，遮罩恢复供用户改为保存。
                self.clipboard()
                    .write_image(image.width(), image.height(), image.as_raw())
                    .map_err(|e| internal(format!("复制到系统剪贴板失败：{}", e)))?;
                outcome.copied = true;
            }
            ExportAction::Save => {
                let png = export::encode_png(&image)?;
                let dir = self.save_directory(ctx, settings);
                let stem =
                    export::timestamped_stem(&settings.file_name_prefix, chrono::Local::now());
                let path = export::write_exclusive(&dir, &stem, &png)?;
                outcome.saved_path = Some(path.to_string_lossy().to_string());
                if settings.copy_after_save {
                    match self.clipboard().write_image(
                        image.width(),
                        image.height(),
                        image.as_raw(),
                    ) {
                        Ok(()) => outcome.copied = true,
                        Err(error) => outcome.clipboard_error = Some(error),
                    }
                }
            }
            ExportAction::Pin => {
                outcome.pin = Some(self.create_pin(ctx, session, selection, &image, settings)?);
            }
        }
        Ok(outcome)
    }

    // ───────────────────────────── 贴图 ─────────────────────────────

    fn create_pin(
        &self,
        ctx: &ModuleContext,
        session: &CaptureSession,
        selection: &SelectionBox,
        image: &image::RgbaImage,
        settings: &ScreenshotSettings,
    ) -> CoreResult<PinSummary> {
        if self.pins.len() >= settings.max_pins {
            return Err(internal(format!(
                "贴图数量已达上限 {}，请先关闭部分贴图",
                settings.max_pins
            )));
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        let record = PinRecord {
            id: id.clone(),
            png: Arc::new(export::encode_png(image)?),
            pixel_width: image.width(),
            pixel_height: image.height(),
            logical: session.monitor.frame_to_logical(selection),
            opacity: settings.default_pin_opacity,
            created_at: chrono::Utc::now().timestamp_millis(),
        };
        let spec = WindowSpec::pin_image(
            &id,
            record.logical.x,
            record.logical.y,
            record.logical.width.max(16.0),
            record.logical.height.max(16.0),
        );
        let summary = record.summary();
        self.pins.insert(record);
        if let Err(error) = ctx.window_pool.open_window(spec) {
            self.pins.remove(&id);
            let _ = ctx.window_pool.destroy_window(&pin_label(&id));
            return Err(internal(format!("创建贴图窗口失败：{}", error)));
        }
        Ok(summary)
    }

    fn pin(&self, pin_id: &str) -> CoreResult<PinRecord> {
        if !is_valid_pin_id(pin_id) {
            return Err(internal("贴图 ID 格式无效"));
        }
        self.pins
            .get(pin_id)
            .ok_or_else(|| internal("贴图不存在或已关闭"))
    }

    pub fn pin_summary(&self, pin_id: &str) -> CoreResult<PinSummary> {
        Ok(self.pin(pin_id)?.summary())
    }

    /// 贴图 PNG（经二进制 IPC 返回）。
    pub fn pin_png(&self, pin_id: &str) -> CoreResult<Arc<Vec<u8>>> {
        Ok(self.pin(pin_id)?.png)
    }

    pub fn list_pins(&self) -> Vec<PinSummary> {
        self.pins.summaries()
    }

    /// 关闭贴图：先销毁窗口，成功后释放 PNG；销毁失败保留记录以便重试。
    pub fn close_pin(&self, ctx: &ModuleContext, pin_id: &str) -> CoreResult<bool> {
        if !is_valid_pin_id(pin_id) {
            return Err(internal("贴图 ID 格式无效"));
        }
        ctx.window_pool.destroy_window(&pin_label(pin_id))?;
        Ok(self.pins.remove(pin_id).is_some())
    }

    /// 原生窗口已由系统关闭（如 Cmd+W）：只释放记录。
    pub fn forget_pin(&self, pin_id: &str) -> bool {
        self.pins.remove(pin_id).is_some()
    }

    /// 关闭全部贴图，返回第一个错误；成功销毁的贴图都会释放。
    pub fn close_all_pins(&self, ctx: &ModuleContext) -> CoreResult<usize> {
        let mut first_error = None;
        let mut closed = 0;
        for id in self.pins.ids() {
            match self.close_pin(ctx, &id) {
                Ok(_) => closed += 1,
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(closed),
        }
    }

    /// 把贴图再次复制为图片。
    pub fn copy_pin(&self, pin_id: &str) -> CoreResult<()> {
        let record = self.pin(pin_id)?;
        let rgba = image::load_from_memory_with_format(&record.png, image::ImageFormat::Png)
            .map_err(|e| internal(format!("贴图解码失败：{}", e)))?
            .to_rgba8();
        self.clipboard()
            .write_image(rgba.width(), rgba.height(), rgba.as_raw())
            .map_err(|e| internal(format!("复制到系统剪贴板失败：{}", e)))
    }

    /// 把贴图另存为新文件（独占创建）。
    pub fn save_pin(&self, ctx: &ModuleContext, pin_id: &str) -> CoreResult<String> {
        let record = self.pin(pin_id)?;
        let settings = self.settings(ctx);
        let dir = self.save_directory(ctx, &settings);
        let stem = export::timestamped_stem(&settings.file_name_prefix, chrono::Local::now());
        let path = export::write_exclusive(&dir, &stem, &record.png)?;
        Ok(path.to_string_lossy().to_string())
    }

    // ───────────────────────────── 其他 ─────────────────────────────

    /// 复制取色结果文本；只接受 `#RRGGBB` 或 `rgb(r, g, b)`，避免借道写入任意内容。
    pub fn copy_color(&self, text: &str) -> CoreResult<()> {
        let text = text.trim();
        if !(config::is_hex_color(text) || is_rgb_text(text)) {
            return Err(internal("颜色格式无效"));
        }
        self.clipboard()
            .write_text(text)
            .map_err(|e| internal(format!("复制颜色失败：{}", e)))
    }

    /// 屏幕录制权限预检（不申请权限）。
    pub fn permission_status(&self) -> PermissionStatus {
        match self.capturer.check_capture_permission() {
            Ok(()) => PermissionStatus {
                granted: true,
                message: None,
                guidance: None,
            },
            Err(error) => {
                let failure = ScreenshotFailure::from_hal(&error);
                PermissionStatus {
                    granted: false,
                    message: Some(failure.message),
                    guidance: failure.guidance,
                }
            }
        }
    }

    pub fn list_monitors(&self) -> CoreResult<Vec<DisplayMonitorInfo>> {
        self.capturer
            .list_monitors()
            .map_err(|e| ScreenshotFailure::from_hal(&e).to_core_error())
    }

    pub fn status(&self, ctx: &ModuleContext) -> ScreenshotStatus {
        let settings = self.settings(ctx);
        let session = self.session.read().as_ref().map(|s| SessionBrief {
            session_id: s.id.clone(),
            monitor_name: s.monitor.name.clone(),
            overlay_visible: self.overlay_visible.load(Ordering::SeqCst),
            created_at: s.created_at,
        });
        let registered = ctx.shortcut_registry.get_by_action(ACTION_CAPTURE);
        ScreenshotStatus {
            session,
            pins: self.pins.summaries(),
            last_export: self.last_export.read().clone(),
            last_error: self.last_error.read().clone(),
            permission: self.permission_status(),
            shortcut: ShortcutBrief {
                action_id: ACTION_CAPTURE.to_string(),
                registered: registered.is_some(),
                display_label: registered.as_ref().map(|r| r.display_label.clone()),
                logical_hotkey: registered.map(|r| r.active_logical_hotkey),
            },
            save_directory: self
                .save_directory(ctx, &settings)
                .to_string_lossy()
                .to_string(),
            settings,
        }
    }

    /// 释放会话与全部贴图；停用与退出共用，可重复调用。
    fn release_all(&self, ctx: &ModuleContext) -> CoreResult<()> {
        // 等待进行中的导出结束；超时也继续清理，避免退出被卡住。
        let gate = self.gate.try_lock_for(SHUTDOWN_GATE_WAIT);
        if gate.is_none() {
            warn!("Screenshot transaction still running during shutdown; forcing cleanup");
        }
        let session_result = self.finish_session_locked(ctx);
        let pins_result = self.close_all_pins(ctx);
        session_result?;
        pins_result.map(|_| ())
    }
}

/// 选择显示器：指定 ID → 主屏 → 第一块。
fn pick_monitor(
    monitors: &[DisplayMonitorInfo],
    monitor_id: Option<u32>,
) -> Option<DisplayMonitorInfo> {
    match monitor_id {
        Some(id) => monitors.iter().find(|m| m.id == id),
        None => monitors
            .iter()
            .find(|m| m.is_primary)
            .or_else(|| monitors.first()),
    }
    .cloned()
}

/// 校验 `rgb(r, g, b)` 文本，每个分量 0–255。
fn is_rgb_text(text: &str) -> bool {
    let Some(inner) = text.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) else {
        return false;
    };
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    parts.len() == 3 && parts.iter().all(|p| p.parse::<u8>().is_ok())
}

impl AppModule for ScreenshotModule {
    fn id(&self) -> &'static str {
        MODULE_ID
    }

    fn name(&self) -> &'static str {
        "增强截图与贴图"
    }

    /// M1 只注册已实现的区域截图；长截图、GIF、OCR 快捷键待对应阶段实现后再声明，避免占用无效按键。
    fn default_shortcuts(&self) -> Vec<ShortcutSpec> {
        vec![ShortcutSpec::new(
            MODULE_ID,
            ACTION_CAPTURE,
            "F1",
            "启动区域截图",
        )]
    }

    fn on_init(&self, ctx: &ModuleContext) -> CoreResult<()> {
        // 按需模块：初始化只校验配置，不抓屏、不建窗口、不启动后台任务。
        let _ = self.settings(ctx);
        *self.last_error.write() = None;
        Ok(())
    }

    fn on_shortcut(&self, action_id: &str, ctx: &ModuleContext) -> CoreResult<()> {
        if action_id != ACTION_CAPTURE {
            return Ok(());
        }
        match self.start_capture(ctx, None, false) {
            Ok(_) => Ok(()),
            Err(error) => {
                // 快捷键触发没有调用方界面：缺权限时打开主面板截图页展示处理步骤。
                let permission_denied = self
                    .last_error
                    .read()
                    .as_ref()
                    .is_some_and(|f| f.kind == error::FailureKind::PermissionDenied);
                if permission_denied {
                    if let Err(open_error) = ctx
                        .window_pool
                        .open_window(WindowSpec::dashboard("/screenshot"))
                    {
                        warn!(%open_error, "Failed to open dashboard for screenshot permission guidance");
                    }
                }
                Err(error)
            }
        }
    }

    fn on_shutdown(&self, ctx: &ModuleContext) -> CoreResult<()> {
        self.release_all(ctx)
    }
}

#[cfg(test)]
#[path = "../tests/unit/lib.rs"]
mod tests;
