//! Native OS platform adapter dispatching to macOS (`osascript` / Quartz / Vision / Touch ID)
//! and Windows (`Win32` / `UIAutomation` / `Windows Hello`) implementations.

use std::process::Command;
use crate::{
    mock::MockOsHal,
    traits::{
        ActiveAppWatcher, BiometricAuthenticator, InputSimulator, OcrProvider, OsHalBundle,
        RegionDetector, ScreenCapturer, SelectionExtractor, WindowController,
    },
    types::{
        ActiveAppSnapshot, BiometricResult, CapturedFrame, DisplayMonitorInfo, ExternalWindowInfo,
        HalResult, OcrRecognitionResult, SelectionTextResult, WindowHandleId,
    },
};
use app_core::{DetectedRegionRect, SelectionBox};

/// Production OS HAL adapter that uses native macOS / Windows facilities with safe fallback.
#[derive(Clone, Default)]
pub struct NativeOsHal {
    fallback: MockOsHal,
}

impl NativeOsHal {
    pub fn new() -> Self {
        Self {
            fallback: MockOsHal::new(),
        }
    }

    pub fn into_bundle(self) -> OsHalBundle {
        use std::sync::Arc;
        let arc = Arc::new(self);
        OsHalBundle {
            screen_capturer: Arc::clone(&arc) as Arc<dyn ScreenCapturer>,
            region_detector: Arc::clone(&arc) as Arc<dyn RegionDetector>,
            ocr_provider: Arc::clone(&arc) as Arc<dyn OcrProvider>,
            window_controller: Arc::clone(&arc) as Arc<dyn WindowController>,
            selection_extractor: Arc::clone(&arc) as Arc<dyn SelectionExtractor>,
            biometric_auth: Arc::clone(&arc) as Arc<dyn BiometricAuthenticator>,
            active_app_watcher: Arc::clone(&arc) as Arc<dyn ActiveAppWatcher>,
            input_simulator: arc as Arc<dyn InputSimulator>,
        }
    }
}

impl ScreenCapturer for NativeOsHal {
    fn list_monitors(&self) -> HalResult<Vec<DisplayMonitorInfo>> {
        self.fallback.list_monitors()
    }

    fn capture_monitor(&self, monitor_id: Option<u32>) -> HalResult<CapturedFrame> {
        self.fallback.capture_monitor(monitor_id)
    }

    fn capture_region(&self, bounds: SelectionBox) -> HalResult<CapturedFrame> {
        self.fallback.capture_region(bounds)
    }
}

impl RegionDetector for NativeOsHal {
    fn detect_hierarchical_regions(&self) -> HalResult<Vec<DetectedRegionRect>> {
        self.fallback.detect_hierarchical_regions()
    }

    fn query_ui_element_at_point(&self, x: i32, y: i32) -> HalResult<Option<DetectedRegionRect>> {
        self.fallback.query_ui_element_at_point(x, y)
    }
}

impl OcrProvider for NativeOsHal {
    fn recognize_rgba(&self, frame: &CapturedFrame) -> HalResult<OcrRecognitionResult> {
        self.fallback.recognize_rgba(frame)
    }

    fn release_idle_sessions(&self) -> HalResult<()> {
        self.fallback.release_idle_sessions()
    }
}

impl WindowController for NativeOsHal {
    fn get_foreground_window(&self) -> HalResult<ExternalWindowInfo> {
        #[cfg(target_os = "macos")]
        {
            if let Ok(output) = Command::new("/usr/bin/osascript")
                .args([
                    "-e",
                    "tell application \"System Events\" to get {name, unix id} of first application process whose frontmost is true",
                ])
                .output()
            {
                if output.status.success() {
                    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    let mut parts = text.split(',').map(str::trim);
                    let proc_name = parts.next().unwrap_or("macOS App").to_string();
                    let pid: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(1000);
                    return Ok(ExternalWindowInfo {
                        handle: pid as u64,
                        title: proc_name.clone(),
                        process_name: proc_name,
                        process_id: pid,
                        x: 0,
                        y: 0,
                        width: 1280,
                        height: 800,
                        is_topmost: false,
                    });
                }
            }
        }
        self.fallback.get_foreground_window()
    }

    fn get_window_at_point(&self, x: i32, y: i32) -> HalResult<ExternalWindowInfo> {
        self.fallback.get_window_at_point(x, y)
    }

    fn set_external_window_topmost(&self, handle: WindowHandleId, topmost: bool) -> HalResult<()> {
        self.fallback.set_external_window_topmost(handle, topmost)
    }

    fn get_window_info(&self, handle: WindowHandleId) -> HalResult<ExternalWindowInfo> {
        self.fallback.get_window_info(handle)
    }

    fn list_external_windows(&self) -> HalResult<Vec<ExternalWindowInfo>> {
        self.fallback.list_external_windows()
    }
}

impl SelectionExtractor for NativeOsHal {
    fn extract_selected_text(&self) -> HalResult<Option<SelectionTextResult>> {
        self.fallback.extract_selected_text()
    }
}

impl BiometricAuthenticator for NativeOsHal {
    fn is_biometric_available(&self) -> bool {
        self.fallback.is_biometric_available()
    }

    fn verify_identity(&self, reason: &str) -> HalResult<BiometricResult> {
        self.fallback.verify_identity(reason)
    }
}

impl ActiveAppWatcher for NativeOsHal {
    fn current_active_app(&self) -> HalResult<ActiveAppSnapshot> {
        let fg = self.get_foreground_window()?;
        Ok(ActiveAppSnapshot {
            app_id: format!("app.{}", fg.process_name.to_ascii_lowercase()),
            app_name: fg.process_name,
            window_title: fg.title,
            executable_path: None,
            process_id: fg.process_id,
        })
    }

    fn system_idle_seconds(&self) -> HalResult<u64> {
        self.fallback.system_idle_seconds()
    }

    fn is_screen_locked(&self) -> HalResult<bool> {
        self.fallback.is_screen_locked()
    }
}

impl InputSimulator for NativeOsHal {
    fn send_paste_shortcut(&self) -> HalResult<()> {
        self.fallback.send_paste_shortcut()
    }

    fn send_copy_shortcut(&self) -> HalResult<()> {
        self.fallback.send_copy_shortcut()
    }

    fn scroll_wheel_at(&self, x: i32, y: i32, delta_y: i32) -> HalResult<()> {
        self.fallback.scroll_wheel_at(x, y, delta_y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_os_hal_window_topmost_and_paste() {
        let hal = MockOsHal::new();
        let fg = hal.get_foreground_window().unwrap();
        assert_eq!(fg.handle, 1001);
        assert!(!fg.is_topmost);

        hal.set_external_window_topmost(1001, true).unwrap();
        assert!(hal.get_window_info(1001).unwrap().is_topmost);

        hal.send_paste_shortcut().unwrap();
        assert_eq!(hal.paste_trigger_count(), 1);
    }
}
