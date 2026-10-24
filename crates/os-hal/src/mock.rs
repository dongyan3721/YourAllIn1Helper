//! Deterministic in-memory mock OS HAL implementation for unit and integration testing.

use crate::{
    traits::{
        ActiveAppWatcher, BiometricAuthenticator, InputSimulator, OcrProvider, OsHalBundle,
        RegionDetector, ScreenCapturer, SelectionExtractor, WindowController,
    },
    types::{
        ActiveAppSnapshot, BiometricResult, CapturedFrame, DisplayMonitorInfo, ExternalWindowInfo,
        HalError, HalResult, OcrBackendKind, OcrLineResult, OcrRecognitionResult, SelectionSource,
        SelectionTextResult, WindowHandleId,
    },
};
use app_core::{DetectedRegionRect, RegionHierarchyLevel, SelectionBox};
use parking_lot::RwLock;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};

/// Thread-safe mock OS HAL adapter with inspectable state for deterministic testing.
#[derive(Clone)]
pub struct MockOsHal {
    windows: Arc<RwLock<HashMap<WindowHandleId, ExternalWindowInfo>>>,
    foreground_handle: Arc<AtomicU64>,
    selected_text: Arc<RwLock<Option<String>>>,
    biometric_available: Arc<AtomicBool>,
    idle_seconds: Arc<AtomicU64>,
    screen_locked: Arc<AtomicBool>,
    paste_count: Arc<AtomicU64>,
    copy_count: Arc<AtomicU64>,
}

impl Default for MockOsHal {
    fn default() -> Self {
        Self::new()
    }
}

impl MockOsHal {
    pub fn new() -> Self {
        let mut initial_windows = HashMap::new();
        initial_windows.insert(
            1001,
            ExternalWindowInfo {
                handle: 1001,
                title: "Visual Studio Code — your_all_in_one".to_string(),
                process_name: "Code".to_string(),
                process_id: 4201,
                x: 80,
                y: 60,
                width: 1440,
                height: 900,
                is_topmost: false,
            },
        );
        initial_windows.insert(
            1002,
            ExternalWindowInfo {
                handle: 1002,
                title: "Terminal — zsh".to_string(),
                process_name: "Terminal".to_string(),
                process_id: 4202,
                x: 200,
                y: 160,
                width: 960,
                height: 640,
                is_topmost: false,
            },
        );

        Self {
            windows: Arc::new(RwLock::new(initial_windows)),
            foreground_handle: Arc::new(AtomicU64::new(1001)),
            selected_text: Arc::new(RwLock::new(Some("Hello All-in-One".to_string()))),
            biometric_available: Arc::new(AtomicBool::new(true)),
            idle_seconds: Arc::new(AtomicU64::new(0)),
            screen_locked: Arc::new(AtomicBool::new(false)),
            paste_count: Arc::new(AtomicU64::new(0)),
            copy_count: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Bundles this `MockOsHal` into an `OsHalBundle` for dependency injection.
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

    pub fn paste_trigger_count(&self) -> u64 {
        self.paste_count.load(Ordering::SeqCst)
    }

    pub fn set_foreground_handle(&self, handle: WindowHandleId) {
        self.foreground_handle.store(handle, Ordering::SeqCst);
    }

    pub fn upsert_window(&self, info: ExternalWindowInfo) {
        self.windows.write().insert(info.handle, info);
    }

    /// Replaces the tracked external window table with live OS windows (preserving `is_topmost` flags).
    pub fn replace_external_windows(
        &self,
        live_windows: Vec<ExternalWindowInfo>,
        fg_handle: Option<WindowHandleId>,
    ) {
        let mut guard = self.windows.write();
        let prev_topmost: HashMap<WindowHandleId, bool> =
            guard.iter().map(|(&h, w)| (h, w.is_topmost)).collect();
        guard.clear();
        for mut win in live_windows {
            if let Some(&was_top) = prev_topmost.get(&win.handle) {
                win.is_topmost = was_top;
            }
            guard.insert(win.handle, win);
        }
        if let Some(fg) = fg_handle {
            self.foreground_handle.store(fg, Ordering::SeqCst);
        }
    }
}

impl ScreenCapturer for MockOsHal {
    fn list_monitors(&self) -> HalResult<Vec<DisplayMonitorInfo>> {
        Ok(vec![DisplayMonitorInfo {
            id: 1,
            name: "Built-in Retina Display".to_string(),
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            scale_factor: 2.0,
            is_primary: true,
        }])
    }

    fn capture_monitor(&self, _monitor_id: Option<u32>) -> HalResult<CapturedFrame> {
        let width = 64;
        let height = 64;
        Ok(CapturedFrame {
            width,
            height,
            scale_factor_milli: 2000,
            rgba_bytes: vec![255u8; (width * height * 4) as usize],
        })
    }

    fn capture_region(&self, bounds: SelectionBox) -> HalResult<CapturedFrame> {
        let w = bounds.width.max(1);
        let h = bounds.height.max(1);
        Ok(CapturedFrame {
            width: w,
            height: h,
            scale_factor_milli: 2000,
            rgba_bytes: vec![255u8; (w * h * 4) as usize],
        })
    }
}

impl RegionDetector for MockOsHal {
    fn detect_hierarchical_regions(&self) -> HalResult<Vec<DetectedRegionRect>> {
        Ok(vec![
            DetectedRegionRect {
                region_id: 1,
                parent_id: None,
                level: RegionHierarchyLevel::Monitor,
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                label: Some("Primary Monitor".to_string()),
            },
            DetectedRegionRect {
                region_id: 2,
                parent_id: Some(1),
                level: RegionHierarchyLevel::TopLevelWindow,
                x: 80,
                y: 60,
                width: 1440,
                height: 900,
                label: Some("Visual Studio Code".to_string()),
            },
        ])
    }

    fn query_ui_element_at_point(&self, x: i32, y: i32) -> HalResult<Option<DetectedRegionRect>> {
        let regions = self.detect_hierarchical_regions()?;
        Ok(app_core::resolve_smart_region_at_point(&regions, x, y, 0))
    }
}

impl OcrProvider for MockOsHal {
    fn recognize_rgba(&self, _frame: &CapturedFrame) -> HalResult<OcrRecognitionResult> {
        Ok(OcrRecognitionResult {
            backend_used: OcrBackendKind::SystemNative,
            full_text: "Sample OCR Text".to_string(),
            lines: vec![OcrLineResult {
                text: "Sample OCR Text".to_string(),
                confidence: 0.99,
                x: 0,
                y: 0,
                width: 120,
                height: 24,
            }],
            elapsed_ms: 12,
        })
    }

    fn release_idle_sessions(&self) -> HalResult<()> {
        Ok(())
    }
}

impl WindowController for MockOsHal {
    fn supports_external_topmost(&self) -> bool {
        true
    }
    fn activate_external_window(&self, handle: WindowHandleId) -> HalResult<()> {
        self.get_window_info(handle)?;
        self.set_foreground_handle(handle);
        Ok(())
    }
    fn get_foreground_window(&self) -> HalResult<ExternalWindowInfo> {
        let handle = self.foreground_handle.load(Ordering::SeqCst);
        self.get_window_info(handle)
    }

    fn get_window_at_point(&self, x: i32, y: i32) -> HalResult<ExternalWindowInfo> {
        let windows = self.windows.read();
        for win in windows.values() {
            let right = win.x.saturating_add(win.width as i32);
            let bottom = win.y.saturating_add(win.height as i32);
            if x >= win.x && x < right && y >= win.y && y < bottom {
                return Ok(win.clone());
            }
        }
        Err(HalError::NotFound(format!(
            "No external window found at ({}, {})",
            x, y
        )))
    }

    fn set_external_window_topmost(&self, handle: WindowHandleId, topmost: bool) -> HalResult<()> {
        let mut windows = self.windows.write();
        let win = windows
            .get_mut(&handle)
            .ok_or_else(|| HalError::NotFound(format!("Window handle {} not found", handle)))?;
        win.is_topmost = topmost;
        Ok(())
    }

    fn get_window_info(&self, handle: WindowHandleId) -> HalResult<ExternalWindowInfo> {
        self.windows
            .read()
            .get(&handle)
            .cloned()
            .ok_or_else(|| HalError::NotFound(format!("Window handle {} not found", handle)))
    }

    fn list_external_windows(&self) -> HalResult<Vec<ExternalWindowInfo>> {
        let mut list: Vec<ExternalWindowInfo> = self.windows.read().values().cloned().collect();
        list.sort_by_key(|w| w.handle);
        Ok(list)
    }
}

impl SelectionExtractor for MockOsHal {
    fn extract_selected_text(&self) -> HalResult<Option<SelectionTextResult>> {
        let guard = self.selected_text.read();
        Ok(guard.as_ref().map(|text| SelectionTextResult {
            text: text.clone(),
            source: SelectionSource::AccessibilityApi,
            source_app_name: Some("Code".to_string()),
        }))
    }
}

impl BiometricAuthenticator for MockOsHal {
    fn is_biometric_available(&self) -> bool {
        self.biometric_available.load(Ordering::SeqCst)
    }

    fn verify_identity(&self, _reason: &str) -> HalResult<BiometricResult> {
        if self.is_biometric_available() {
            Ok(BiometricResult::Verified)
        } else {
            Ok(BiometricResult::NotEnrolled)
        }
    }
}

impl ActiveAppWatcher for MockOsHal {
    fn current_active_app(&self) -> HalResult<ActiveAppSnapshot> {
        let fg = self.get_foreground_window()?;
        Ok(ActiveAppSnapshot {
            app_id: format!("com.mock.{}", fg.process_name.to_ascii_lowercase()),
            app_name: fg.process_name,
            window_title: fg.title,
            executable_path: None,
            process_id: fg.process_id,
        })
    }

    fn system_idle_seconds(&self) -> HalResult<u64> {
        Ok(self.idle_seconds.load(Ordering::SeqCst))
    }

    fn is_screen_locked(&self) -> HalResult<bool> {
        Ok(self.screen_locked.load(Ordering::SeqCst))
    }
}

impl InputSimulator for MockOsHal {
    fn paste_trigger_count(&self) -> u64 {
        MockOsHal::paste_trigger_count(self)
    }
    fn send_paste_shortcut(&self) -> HalResult<()> {
        self.paste_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn send_copy_shortcut(&self) -> HalResult<()> {
        self.copy_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn scroll_wheel_at(&self, _x: i32, _y: i32, _delta_y: i32) -> HalResult<()> {
        Ok(())
    }
}
