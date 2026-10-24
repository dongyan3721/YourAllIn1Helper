//! Native cross-platform OS adapter.
//!
//! Production uses `xcap` for window/screen information and `enigo` for synthetic input.
//! `NativeOsHal::mock_only()` keeps deterministic tests without changing production wiring.

use std::{
    process::Command,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use app_core::{DetectedRegionRect, RegionHierarchyLevel, SelectionBox};
use enigo::{Axis, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use xcap::{Monitor, Window};

use crate::{
    mock::MockOsHal,
    traits::{
        ActiveAppWatcher, BiometricAuthenticator, InputSimulator, OcrProvider, OsHalBundle,
        RegionDetector, ScreenCapturer, SelectionExtractor, WindowController,
    },
    types::{
        ActiveAppSnapshot, BiometricResult, CapturedFrame, DisplayMonitorInfo, ExternalWindowInfo,
        HalError, HalResult, OcrRecognitionResult, SelectionTextResult, WindowHandleId,
    },
};

fn native_error(error: impl std::fmt::Display) -> HalError {
    HalError::NativeCallFailed(error.to_string())
}

/// Production OS HAL adapter with an explicit deterministic mock mode for tests.
#[derive(Clone)]
pub struct NativeOsHal {
    fallback: MockOsHal,
    native_enabled: bool,
    paste_triggers: Arc<AtomicUsize>,
}

impl Default for NativeOsHal {
    fn default() -> Self {
        Self::new()
    }
}

impl NativeOsHal {
    pub fn new() -> Self {
        Self {
            fallback: MockOsHal::new(),
            native_enabled: true,
            paste_triggers: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Uses only deterministic mock windows and input, intended for unit/smoke tests.
    pub fn mock_only() -> Self {
        Self {
            fallback: MockOsHal::new(),
            native_enabled: false,
            paste_triggers: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn is_mock_only(&self) -> bool {
        !self.native_enabled
    }

    pub fn mock(&self) -> &MockOsHal {
        &self.fallback
    }

    pub fn paste_trigger_count(&self) -> usize {
        if self.native_enabled {
            self.paste_triggers.load(Ordering::SeqCst)
        } else {
            self.fallback.paste_trigger_count() as usize
        }
    }

    pub fn into_bundle(self) -> OsHalBundle {
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

    fn native_windows(&self) -> HalResult<Vec<ExternalWindowInfo>> {
        let own_pid = std::process::id();
        let windows = Window::all().map_err(native_error)?;
        let mut result = Vec::new();
        for window in windows {
            let pid = window.pid().unwrap_or(0);
            if pid == own_pid || window.is_minimized().unwrap_or(false) {
                continue;
            }
            let width = window.width().unwrap_or(0);
            let height = window.height().unwrap_or(0);
            if width < 24 || height < 24 {
                continue;
            }
            let process_name = window
                .app_name()
                .unwrap_or_else(|_| "Unknown App".to_string());
            let title = window.title().unwrap_or_default();
            result.push(ExternalWindowInfo {
                handle: window.id().unwrap_or(0) as u64,
                title: if title.is_empty() {
                    process_name.clone()
                } else {
                    title
                },
                process_name,
                process_id: pid,
                x: window.x().unwrap_or(0),
                y: window.y().unwrap_or(0),
                width,
                height,
                is_topmost: false,
            });
        }
        Ok(result)
    }

    fn find_native_window(&self, handle: WindowHandleId) -> HalResult<ExternalWindowInfo> {
        self.native_windows()?
            .into_iter()
            .find(|window| window.handle == handle)
            .ok_or_else(|| HalError::NotFound(format!("External window {}", handle)))
    }

    fn activate_native_window(&self, handle: WindowHandleId) -> HalResult<()> {
        let info = self.find_native_window(handle)?;
        #[cfg(target_os = "macos")]
        {
            // LaunchServices activation does not require Apple Events / Automation permission.
            let status = Command::new("/usr/bin/open")
                .args(["-a", &info.process_name])
                .status()
                .map_err(native_error)?;
            if !status.success() {
                return Err(HalError::NativeCallFailed(format!(
                    "Unable to activate application '{}'",
                    info.process_name
                )));
            }
            return Ok(());
        }
        #[cfg(target_os = "windows")]
        {
            use windows::Win32::{Foundation::HWND, UI::WindowsAndMessaging::SetForegroundWindow};
            unsafe {
                SetForegroundWindow(HWND(info.handle as *mut std::ffi::c_void))
                    .map_err(native_error)?;
            }
            return Ok(());
        }
        #[allow(unreachable_code)]
        Err(HalError::Unsupported("external_window_activation"))
    }

    /// Activates a real external window (used by the Tauri pinned-mirror view).
    pub fn activate_external_window(&self, handle: WindowHandleId) -> HalResult<()> {
        if !self.native_enabled {
            return self.fallback.set_external_window_topmost(handle, true);
        }
        self.activate_native_window(handle)
    }

    /// Captures and JPEG-encodes a real external window for the Tauri pinned-mirror view.
    pub fn capture_window_data_url(&self, handle: WindowHandleId) -> HalResult<String> {
        use base64::{engine::general_purpose::STANDARD, Engine as _};

        if !self.native_enabled {
            return Err(HalError::Unsupported("mock_window_capture"));
        }
        let windows = Window::all().map_err(native_error)?;
        let window = windows
            .into_iter()
            .find(|window| window.id().map(|id| id as u64 == handle).unwrap_or(false))
            .ok_or_else(|| HalError::NotFound(format!("External window {}", handle)))?;
        let image = window.capture_image().map_err(native_error)?;
        let dynamic = image::DynamicImage::ImageRgba8(image);
        let resized = if dynamic.width() > 960 {
            dynamic.resize(960, u32::MAX, image::imageops::FilterType::Triangle)
        } else {
            dynamic
        };
        let mut encoded = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 72)
            .encode_image(&resized)
            .map_err(native_error)?;
        Ok(format!(
            "data:image/jpeg;base64,{}",
            STANDARD.encode(encoded)
        ))
    }

    fn send_modified_key(&self, key_char: char) -> HalResult<()> {
        let mut enigo = Enigo::new(&Settings::default()).map_err(native_error)?;
        #[cfg(target_os = "macos")]
        let modifier = Key::Meta;
        #[cfg(not(target_os = "macos"))]
        let modifier = Key::Control;
        enigo
            .key(modifier, Direction::Press)
            .map_err(native_error)?;
        enigo
            .key(Key::Unicode(key_char), Direction::Click)
            .map_err(native_error)?;
        enigo
            .key(modifier, Direction::Release)
            .map_err(native_error)?;
        Ok(())
    }
}

impl ScreenCapturer for NativeOsHal {
    fn list_monitors(&self) -> HalResult<Vec<DisplayMonitorInfo>> {
        if !self.native_enabled {
            return self.fallback.list_monitors();
        }
        let monitors = Monitor::all().map_err(native_error)?;
        Ok(monitors
            .into_iter()
            .map(|monitor| DisplayMonitorInfo {
                id: monitor.id().unwrap_or(0),
                name: monitor
                    .friendly_name()
                    .or_else(|_| monitor.name())
                    .unwrap_or_else(|_| "Monitor".to_string()),
                x: monitor.x().unwrap_or(0),
                y: monitor.y().unwrap_or(0),
                width: monitor.width().unwrap_or(0),
                height: monitor.height().unwrap_or(0),
                scale_factor: monitor.scale_factor().unwrap_or(1.0) as f64,
                is_primary: monitor.is_primary().unwrap_or(false),
            })
            .collect())
    }

    fn capture_monitor(&self, monitor_id: Option<u32>) -> HalResult<CapturedFrame> {
        if !self.native_enabled {
            return self.fallback.capture_monitor(monitor_id);
        }
        let monitors = Monitor::all().map_err(native_error)?;
        let monitor = match monitor_id {
            Some(id) => monitors
                .into_iter()
                .find(|m| m.id().map(|value| value == id).unwrap_or(false)),
            None => monitors
                .into_iter()
                .find(|m| m.is_primary().unwrap_or(false)),
        }
        .or_else(|| Monitor::all().ok()?.into_iter().next())
        .ok_or_else(|| HalError::NotFound("No available monitor".to_string()))?;
        let image = monitor.capture_image().map_err(native_error)?;
        Ok(CapturedFrame {
            width: image.width(),
            height: image.height(),
            scale_factor_milli: (monitor.scale_factor().unwrap_or(1.0) * 1000.0) as u32,
            rgba_bytes: image.into_raw(),
        })
    }

    fn capture_region(&self, bounds: SelectionBox) -> HalResult<CapturedFrame> {
        if !self.native_enabled {
            return self.fallback.capture_region(bounds);
        }
        let monitor = Monitor::from_point(bounds.x, bounds.y).map_err(native_error)?;
        let image = monitor
            .capture_region(
                bounds.x.max(0) as u32,
                bounds.y.max(0) as u32,
                bounds.width,
                bounds.height,
            )
            .map_err(native_error)?;
        Ok(CapturedFrame {
            width: image.width(),
            height: image.height(),
            scale_factor_milli: (monitor.scale_factor().unwrap_or(1.0) * 1000.0) as u32,
            rgba_bytes: image.into_raw(),
        })
    }
}

impl RegionDetector for NativeOsHal {
    fn detect_hierarchical_regions(&self) -> HalResult<Vec<DetectedRegionRect>> {
        if !self.native_enabled {
            return self.fallback.detect_hierarchical_regions();
        }
        let mut regions = Vec::new();
        for (index, monitor) in self.list_monitors()?.into_iter().enumerate() {
            regions.push(DetectedRegionRect {
                region_id: index as u32 + 1,
                parent_id: None,
                level: RegionHierarchyLevel::Monitor,
                x: monitor.x,
                y: monitor.y,
                width: monitor.width,
                height: monitor.height,
                label: Some(monitor.name),
            });
        }
        for (index, window) in self.native_windows()?.into_iter().enumerate() {
            regions.push(DetectedRegionRect {
                region_id: 10_000 + index as u32,
                parent_id: Some(1),
                level: RegionHierarchyLevel::TopLevelWindow,
                x: window.x,
                y: window.y,
                width: window.width,
                height: window.height,
                label: Some(format!("{} — {}", window.process_name, window.title)),
            });
        }
        Ok(regions)
    }

    fn query_ui_element_at_point(&self, x: i32, y: i32) -> HalResult<Option<DetectedRegionRect>> {
        if !self.native_enabled {
            return self.fallback.query_ui_element_at_point(x, y);
        }
        Ok(self
            .get_window_at_point(x, y)
            .ok()
            .map(|window| DetectedRegionRect {
                region_id: window.handle as u32,
                parent_id: None,
                level: RegionHierarchyLevel::TopLevelWindow,
                x: window.x,
                y: window.y,
                width: window.width,
                height: window.height,
                label: Some(window.title),
            }))
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
        if !self.native_enabled {
            return self.fallback.get_foreground_window();
        }
        let windows = Window::all().map_err(native_error)?;
        let own_pid = std::process::id();
        if let Some(window) = windows.iter().find(|window| {
            window.is_focused().unwrap_or(false) && window.pid().unwrap_or(0) != own_pid
        }) {
            let handle = window.id().map_err(native_error)? as u64;
            return self.find_native_window(handle);
        }
        self.native_windows()?
            .into_iter()
            .next()
            .ok_or_else(|| HalError::NotFound("No foreground external window".to_string()))
    }

    fn get_window_at_point(&self, x: i32, y: i32) -> HalResult<ExternalWindowInfo> {
        if !self.native_enabled {
            return self.fallback.get_window_at_point(x, y);
        }
        self.native_windows()?
            .into_iter()
            .find(|window| {
                x >= window.x
                    && x < window.x + window.width as i32
                    && y >= window.y
                    && y < window.y + window.height as i32
            })
            .ok_or_else(|| HalError::NotFound(format!("No window at ({}, {})", x, y)))
    }

    fn set_external_window_topmost(&self, handle: WindowHandleId, topmost: bool) -> HalResult<()> {
        if !self.native_enabled {
            return self.fallback.set_external_window_topmost(handle, topmost);
        }
        #[cfg(target_os = "windows")]
        {
            use windows::Win32::{
                Foundation::HWND,
                UI::WindowsAndMessaging::{
                    SetWindowPos, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE,
                    SWP_NOSIZE,
                },
            };
            unsafe {
                SetWindowPos(
                    HWND(handle as *mut std::ffi::c_void),
                    if topmost {
                        HWND_TOPMOST
                    } else {
                        HWND_NOTOPMOST
                    },
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                )
                .map_err(native_error)?;
            }
            return Ok(());
        }
        #[cfg(target_os = "macos")]
        {
            // macOS has no supported API to change another process' CGWindow level.
            // The Tauri host renders the pinned live mirror. Pinning itself must not require
            // Accessibility permission; only explicit activation does.
            let _ = topmost;
            return Ok(());
        }
        #[allow(unreachable_code)]
        Err(HalError::Unsupported("external_window_topmost"))
    }

    fn get_window_info(&self, handle: WindowHandleId) -> HalResult<ExternalWindowInfo> {
        if !self.native_enabled {
            return self.fallback.get_window_info(handle);
        }
        self.find_native_window(handle)
    }

    fn list_external_windows(&self) -> HalResult<Vec<ExternalWindowInfo>> {
        if !self.native_enabled {
            return self.fallback.list_external_windows();
        }
        self.native_windows()
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
        if !self.native_enabled {
            return self.fallback.send_paste_shortcut();
        }
        // Give the previous application time to regain focus after the transient popup closes.
        std::thread::sleep(Duration::from_millis(140));
        self.send_modified_key('v')?;
        self.paste_triggers.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn send_copy_shortcut(&self) -> HalResult<()> {
        if !self.native_enabled {
            return self.fallback.send_copy_shortcut();
        }
        self.send_modified_key('c')
    }

    fn scroll_wheel_at(&self, x: i32, y: i32, delta_y: i32) -> HalResult<()> {
        if !self.native_enabled {
            return self.fallback.scroll_wheel_at(x, y, delta_y);
        }
        let mut enigo = Enigo::new(&Settings::default()).map_err(native_error)?;
        enigo
            .move_mouse(x, y, Coordinate::Abs)
            .map_err(native_error)?;
        enigo.scroll(delta_y, Axis::Vertical).map_err(native_error)
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
