//! Core bridge traits defining the OS Hardware Abstraction Layer (`os-hal`).

use std::sync::Arc;
use app_core::{DetectedRegionRect, SelectionBox};
use crate::types::{
    ActiveAppSnapshot, BiometricResult, CapturedFrame, DisplayMonitorInfo, ExternalWindowInfo,
    HalResult, OcrRecognitionResult, SelectionTextResult, WindowHandleId,
};

/// 1. Screen & window frame capture adapter (`mod-screenshot`, `mod-screen-ink`).
pub trait ScreenCapturer: Send + Sync + 'static {
    /// Lists all connected display monitors and their physical coordinates.
    fn list_monitors(&self) -> HalResult<Vec<DisplayMonitorInfo>>;
    /// Captures a frozen RGBA8 frame of the specified monitor (or primary monitor if `None`).
    fn capture_monitor(&self, monitor_id: Option<u32>) -> HalResult<CapturedFrame>;
    /// Captures a sub-rectangle of the screen in physical coordinates.
    fn capture_region(&self, bounds: SelectionBox) -> HalResult<CapturedFrame>;
}

/// 2. Snipaste-style L1 (window tree) & L2 (UIAutomation / AXUIElement) region detector (`mod-screenshot`).
pub trait RegionDetector: Send + Sync + 'static {
    /// Enumerates visible windows and sub-windows in Z-order along with accessible UI containers
    /// to pre-populate the hierarchical region tree when freezing the screen.
    fn detect_hierarchical_regions(&self) -> HalResult<Vec<DetectedRegionRect>>;

    /// Queries the finest-grained UI control element at physical cursor position `(x, y)` on demand.
    fn query_ui_element_at_point(&self, x: i32, y: i32) -> HalResult<Option<DetectedRegionRect>>;
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
}

/// 8. Synthetic input simulator (`mod-clipboard` quick paste, `mod-screenshot` scroll capture).
pub trait InputSimulator: Send + Sync + 'static {
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
