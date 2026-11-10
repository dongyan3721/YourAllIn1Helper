//! Core bridge traits defining the OS Hardware Abstraction Layer (`os-hal`).

use crate::types::{
    ActiveAppSnapshot, BiometricResult, CapturedFrame, DisplayMonitorInfo, ExternalWindowInfo,
    HalError, HalResult, OcrRecognitionResult, SelectionTextResult, WindowHandleId,
};
use app_core::{DetectedRegionRect, SelectionBox};
use std::sync::Arc;

/// 1. Screen & window frame capture adapter (`mod-screenshot`, `mod-screen-ink`).
pub trait ScreenCapturer: Send + Sync + 'static {
    /// Lists all connected display monitors and their physical coordinates.
    fn list_monitors(&self) -> HalResult<Vec<DisplayMonitorInfo>>;
    /// Captures a frozen RGBA8 frame of the specified monitor (or primary monitor if `None`).
    fn capture_monitor(&self, monitor_id: Option<u32>) -> HalResult<CapturedFrame>;
    /// 抓取全局桌面单位矩形（与 `list_monitors` 的 x/y/宽高同一空间）。
    /// macOS 为逻辑点，Windows 为物理像素。不要传入冻结帧的物理像素。
    fn capture_region(&self, bounds: SelectionBox) -> HalResult<CapturedFrame>;

    /// 同 [`capture_region`]，但 `below_window_id != 0` 时只采集该窗口之下的画面。
    /// 用于 GIF / 长截图避开自身遮罩。默认忽略窗口号，退回普通区域抓取。
    /// Windows 的遮罩是 64 位 HWND，无法由 `u32` 表达；Windows 调用方应先通过
    /// [`Self::set_capture_exclusion`] 登记句柄，再以 0 调用本方法复用已登记的排除。
    fn capture_region_below(
        &self,
        bounds: SelectionBox,
        below_window_id: u32,
    ) -> HalResult<CapturedFrame> {
        let _ = below_window_id;
        self.capture_region(bounds)
    }

    /// 登记一个原生窗口，使后续区域抓取不再包含它的内容。
    ///
    /// Windows 使用 `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`：遮罩从抓屏中消失，
    /// 露出底下真实画面；macOS 的排除由 [`Self::capture_region_excluding`] 的窗口号参数完成。
    /// 默认实现不维护原生状态，调用方仍可显式传入排除标识。
    fn set_capture_exclusion(&self, _handle: Option<WindowHandleId>) -> HalResult<()> {
        Ok(())
    }

    /// 当前已登记的原生排除窗口（64 位，不截断 HWND）。
    fn active_capture_exclusion(&self) -> Option<WindowHandleId> {
        None
    }

    /// 实时录制前检查原生遮罩排除是否就绪；普通冻结截图不依赖此能力。
    /// 测试适配器及不要求预登记的平台保持默认行为。
    fn prepare_live_capture(&self) -> HalResult<()> {
        Ok(())
    }

    /// 实时采集结束后恢复窗口的正常捕获；保留登记，允许当前选区重新录制。
    fn finish_live_capture(&self) -> HalResult<()> {
        Ok(())
    }

    /// 抓取区域并排除 `exclusion` 指定的原生窗口。
    ///
    /// 能放进 `u32` 的非零标识仍转交 [`Self::capture_region_below`]（macOS `CGWindowID` 属于这一类）。
    /// 超出 `u32` 的句柄必须报错：静默退回 [`Self::capture_region`] 会把遮罩重新拍进画面。
    /// Windows 原生实现自行覆盖本方法，用完整 64 位 HWND 做捕获排除。
    fn capture_region_excluding(
        &self,
        bounds: SelectionBox,
        exclusion: Option<WindowHandleId>,
    ) -> HalResult<CapturedFrame> {
        match exclusion {
            Some(handle) if handle != 0 => {
                let window_id = u32::try_from(handle).map_err(|_| {
                    HalError::NativeCallFailed(format!(
                        "抓屏排除标识无法用 32 位窗口号表示：{handle}"
                    ))
                })?;
                self.capture_region_below(bounds, window_id)
            }
            _ => self.capture_region(bounds),
        }
    }

    /// 抓屏前的权限预检：只检查、不申请，也不修改系统安全设置。
    /// 没有独立屏幕录制授权概念的平台（如 Windows 桌面会话）保持默认 `Ok`；
    /// macOS 未授权时系统仍会返回仅含壁纸的画面，因此必须显式预检并返回 `PermissionDenied`。
    fn check_capture_permission(&self) -> HalResult<()> {
        Ok(())
    }

    /// 由用户点击截图触发。macOS 会在此时弹出系统的屏幕录制授权；
    /// 默认复用被动检查，保留显式测试适配器的拒绝行为。
    fn request_capture_permission(&self) -> HalResult<()> {
        self.check_capture_permission()
    }
}

/// 2. Snipaste-style L1 (window tree) & L2 (UIAutomation / AXUIElement) region detector (`mod-screenshot`).
pub trait RegionDetector: Send + Sync + 'static {
    /// Enumerates visible windows and sub-windows in Z-order along with accessible UI containers
    /// to pre-populate the hierarchical region tree when freezing the screen.
    fn detect_hierarchical_regions(&self) -> HalResult<Vec<DetectedRegionRect>>;

    /// Queries the finest-grained UI control element at physical cursor position `(x, y)` on demand.
    fn query_ui_element_at_point(&self, x: i32, y: i32) -> HalResult<Option<DetectedRegionRect>>;

    /// L2：光标处最前方外部窗口内的无障碍控件层级链（叶子在前、窗口之下的容器在后），
    /// 坐标为桌面单位。默认表示平台未接入，不返回虚构控件。
    fn query_ui_hierarchy_at_point(&self, _x: i32, _y: i32) -> HalResult<Vec<DetectedRegionRect>> {
        Err(HalError::Unsupported("ui_element_hierarchy"))
    }

    /// L2 能力预检：缺辅助功能权限时返回 `PermissionDenied`（只检查，不弹出授权）。
    fn check_ui_hierarchy_permission(&self) -> HalResult<()> {
        Err(HalError::Unsupported("ui_element_hierarchy"))
    }
}

/// 3. OCR inference engine (`SystemNative` first -> `OnnxGpu` -> `OnnxNpu` -> `OnnxCpu`).
pub trait OcrProvider: Send + Sync + 'static {
    /// Runs text recognition on an RGBA8 image buffer.
    fn recognize_rgba(&self, frame: &CapturedFrame) -> HalResult<OcrRecognitionResult>;
    /// Releases any cached custom ONNX sessions from RAM/VRAM after idle timeout.
    fn release_idle_sessions(&self) -> HalResult<()>;
}

/// 4. External window manipulation & topmost controller (`mod-window-pin`, `mod-screen-ink`).
pub trait WindowController: Send + Sync + 'static {
    /// Whether this adapter can keep another application's original window topmost.
    fn supports_external_topmost(&self) -> bool {
        false
    }
    fn activate_external_window(&self, _handle: WindowHandleId) -> HalResult<()> {
        Err(HalError::Unsupported("external_window_activation"))
    }
    fn capture_window_data_url(&self, _handle: WindowHandleId) -> HalResult<String> {
        Err(HalError::Unsupported("external_window_capture"))
    }
    /// Returns metadata for the currently active foreground window.
    fn get_foreground_window(&self) -> HalResult<ExternalWindowInfo>;
    /// Resolves the top-level external window located at physical screen coordinates `(x, y)`.
    fn get_window_at_point(&self, x: i32, y: i32) -> HalResult<ExternalWindowInfo>;
    /// Sets or clears the system topmost (`HWND_TOPMOST` / floating window level) state for `handle`.
    fn set_external_window_topmost(&self, handle: WindowHandleId, topmost: bool) -> HalResult<()>;
    /// Queries details of a specific external window handle.
    fn get_window_info(&self, handle: WindowHandleId) -> HalResult<ExternalWindowInfo>;
    /// Lists all visible external windows available on the desktop.
    fn list_external_windows(&self) -> HalResult<Vec<ExternalWindowInfo>> {
        Ok(Vec::new())
    }
}

/// 5. Global selected text extractor (`mod-translator`).
pub trait SelectionExtractor: Send + Sync + 'static {
    /// Whether this adapter can actually read a text selection. Features must not
    /// advertise `$AIO_SELECTION` when this returns `false`.
    fn is_available(&self) -> bool {
        false
    }
    /// Extracts currently highlighted text using native Accessibility API first,
    /// falling back to simulated `Ctrl+C`/`Cmd+C` if necessary.
    fn extract_selected_text(&self) -> HalResult<Option<SelectionTextResult>>;
}

/// 6. Biometric and device-owner authentication adapter (`mod-password-vault`).
pub trait BiometricAuthenticator: Send + Sync + 'static {
    /// Checks whether Windows Hello or macOS Touch ID / device authentication is available.
    fn is_biometric_available(&self) -> bool;
    /// Prompts the user with a localized reason string to verify identity.
    fn verify_identity(&self, reason: &str) -> HalResult<BiometricResult>;
}

/// 7. Foreground active application & system idle/lock state watcher (`mod-app-usage`).
pub trait ActiveAppWatcher: Send + Sync + 'static {
    /// Queries the current foreground application snapshot.
    fn current_active_app(&self) -> HalResult<ActiveAppSnapshot>;
    /// Returns the number of seconds since the last keyboard or mouse input event.
    fn system_idle_seconds(&self) -> HalResult<u64>;
    /// Returns `true` if the workstation screen is currently locked or asleep.
    fn is_screen_locked(&self) -> HalResult<bool>;
    /// 查询默认输出设备当前是否有音频流在运行（只看是否播放，不读取音频内容）。
    /// 未实现或不可用的平台必须返回 `Unsupported`，不得用 `false` 冒充真实能力。
    fn default_output_audio_active(&self) -> HalResult<bool> {
        Err(HalError::Unsupported("default_output_audio_activity"))
    }
    /// 订阅系统锁屏/息屏/睡眠事件；实现方必须保证重复启动幂等。
    /// 默认表示平台没有事件源，调用方应继续使用严格轮询语义。
    fn start_usage_event_monitor(&self) -> HalResult<()> {
        Err(HalError::Unsupported("usage_event_monitor"))
    }
    /// 取消订阅并释放事件观察者与后台线程；未启动时调用也必须安全。
    fn stop_usage_event_monitor(&self) {}
}

/// 8. Synthetic input simulator (`mod-clipboard` quick paste, `mod-screenshot` scroll capture).
pub trait InputSimulator: Send + Sync + 'static {
    /// Remember the destination before a clipboard popup takes foreground focus.
    fn remember_paste_target(&self) -> HalResult<()> {
        Ok(())
    }
    /// Drop a remembered destination whose popup session ended without pasting, so a later
    /// paste from another surface can never be sent to a window the user already left.
    fn forget_paste_target(&self) {}
    /// Permission preflight before clipboard/window mutations; may show system consent UI.
    fn check_input_permission(&self) -> HalResult<()> {
        Ok(())
    }
    fn paste_trigger_count(&self) -> u64 {
        0
    }
    /// Sends a synthetic `Ctrl+V` (Windows/Linux) or `Cmd+V` (macOS) keystroke to the active window.
    fn send_paste_shortcut(&self) -> HalResult<()>;
    /// Sends a synthetic `Ctrl+C` (Windows/Linux) or `Cmd+C` (macOS) keystroke.
    fn send_copy_shortcut(&self) -> HalResult<()>;
    /// Sends vertical mouse wheel scroll delta at `(x, y)` (used by long screenshot stitching).
    fn scroll_wheel_at(&self, x: i32, y: i32, delta_y: i32) -> HalResult<()>;
}

/// Dependency-injected bundle holding all OS HAL trait adapters.
#[derive(Clone)]
pub struct OsHalBundle {
    pub screen_capturer: Arc<dyn ScreenCapturer>,
    pub region_detector: Arc<dyn RegionDetector>,
    pub ocr_provider: Arc<dyn OcrProvider>,
    pub window_controller: Arc<dyn WindowController>,
    pub selection_extractor: Arc<dyn SelectionExtractor>,
    pub biometric_auth: Arc<dyn BiometricAuthenticator>,
    pub active_app_watcher: Arc<dyn ActiveAppWatcher>,
    pub input_simulator: Arc<dyn InputSimulator>,
}

impl OsHalBundle {
    /// The composition root supplies adapters; features receive only the capabilities they need.
    pub fn from_adapter<T>(adapter: T) -> Self
    where
        T: ScreenCapturer
            + RegionDetector
            + OcrProvider
            + WindowController
            + SelectionExtractor
            + BiometricAuthenticator
            + ActiveAppWatcher
            + InputSimulator,
    {
        let adapter = Arc::new(adapter);
        Self {
            screen_capturer: adapter.clone(),
            region_detector: adapter.clone(),
            ocr_provider: adapter.clone(),
            window_controller: adapter.clone(),
            selection_extractor: adapter.clone(),
            biometric_auth: adapter.clone(),
            active_app_watcher: adapter.clone(),
            input_simulator: adapter,
        }
    }
}

#[cfg(test)]
mod capture_exclusion_default_tests {
    use super::ScreenCapturer;
    use crate::{CapturedFrame, DisplayMonitorInfo, HalResult, WindowHandleId};
    use app_core::SelectionBox;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingCapturer {
        region_calls: AtomicUsize,
        below_calls: AtomicUsize,
    }

    impl CountingCapturer {
        fn new() -> Self {
            Self {
                region_calls: AtomicUsize::new(0),
                below_calls: AtomicUsize::new(0),
            }
        }
    }

    impl ScreenCapturer for CountingCapturer {
        fn list_monitors(&self) -> HalResult<Vec<DisplayMonitorInfo>> {
            Ok(vec![])
        }

        fn capture_monitor(&self, _monitor_id: Option<u32>) -> HalResult<CapturedFrame> {
            Err(crate::HalError::NativeCallFailed(
                "monitor capture is not used".to_string(),
            ))
        }

        fn capture_region(&self, bounds: SelectionBox) -> HalResult<CapturedFrame> {
            self.region_calls.fetch_add(1, Ordering::SeqCst);
            let pixels = (bounds.width.max(1) * bounds.height.max(1) * 4) as usize;
            Ok(CapturedFrame {
                width: bounds.width.max(1),
                height: bounds.height.max(1),
                scale_factor_milli: 1000,
                rgba_bytes: vec![9, 9, 9, 255].repeat(pixels / 4),
            })
        }

        fn capture_region_below(
            &self,
            bounds: SelectionBox,
            _below_window_id: u32,
        ) -> HalResult<CapturedFrame> {
            self.below_calls.fetch_add(1, Ordering::SeqCst);
            self.capture_region(bounds)
        }
    }

    fn bounds() -> SelectionBox {
        SelectionBox {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
        }
    }

    #[test]
    fn default_capture_exclusion_rejects_handle_wider_than_u32() {
        let capturer = CountingCapturer::new();
        // 与 Windows HWND 保留测试相同的高位形态，不能被当成普通抓屏静默丢掉。
        let handle: WindowHandleId = 0x0000_7FFE_1234_5678;
        let error = capturer
            .capture_region_excluding(bounds(), Some(handle))
            .expect_err("oversized exclusion must fail")
            .to_string();

        assert!(
            error.contains("无法用 32 位窗口号表示"),
            "unexpected error: {error}"
        );
        assert!(
            error.contains(&handle.to_string()),
            "error must keep the full handle, got {error}"
        );
        assert_eq!(capturer.region_calls.load(Ordering::SeqCst), 0);
        assert_eq!(capturer.below_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn default_capture_exclusion_keeps_u32_window_ids() {
        let capturer = CountingCapturer::new();
        capturer
            .capture_region_excluding(bounds(), Some(u32::MAX as WindowHandleId))
            .expect("CGWindowID-sized handles still delegate");
        assert_eq!(capturer.below_calls.load(Ordering::SeqCst), 1);

        capturer
            .capture_region_excluding(bounds(), Some(0))
            .expect("zero means no exclusion");
        capturer
            .capture_region_excluding(bounds(), None)
            .expect("missing exclusion uses a normal capture");
        assert_eq!(capturer.below_calls.load(Ordering::SeqCst), 1);
        assert_eq!(capturer.region_calls.load(Ordering::SeqCst), 3);
    }
}
