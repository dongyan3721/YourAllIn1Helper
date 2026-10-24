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
        #[cfg(target_os = "macos")]
        {
            return self.native_windows_macos();
        }
        #[cfg(not(target_os = "macos"))]
        {
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
    }

    #[cfg(target_os = "macos")]
    /// macOS 原生高精度窗口枚举实现：
    /// 解决旧版 xcap 因未授权屏幕录制权限导致丢弃无标题窗口、仅残留 Window Server 全局对象的严重缺陷。
    /// 1. 严格过滤 kCGWindowLayer == 0（仅保留常规应用主窗口，剔除 Layer 24 的 WindowServer/菜单栏与 Layer 20 的 Dock 等系统幽灵层）；
    /// 2. 结合 NSRunningApplication 获取本地化应用程序名称；
    /// 3. 当窗口标题被系统保护时，自动回退至应用程序名称，不丢弃窗口。
    fn native_windows_macos(&self) -> HalResult<Vec<ExternalWindowInfo>> {
        use objc2_app_kit::NSRunningApplication;
        use objc2_core_foundation::{CFDictionary, CFNumber, CFNumberType, CFString, CGRect};
        use objc2_core_graphics::{
            CGRectMakeWithDictionaryRepresentation, CGWindowListCopyWindowInfo, CGWindowListOption,
        };
        use std::ffi::c_void;

        let own_pid = std::process::id();
        let mut result = Vec::new();

        unsafe {
            // 获取所有屏幕上显示的窗口列表（排除桌面元素）
            let cf_array = match CGWindowListCopyWindowInfo(
                CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements,
                0,
            ) {
                Some(arr) => arr,
                None => return Ok(result),
            };

            let count = cf_array.count();
            for i in 0..count {
                let dict_ref = cf_array.value_at_index(i) as *const CFDictionary;
                if dict_ref.is_null() {
                    continue;
                }
                let dict = &*dict_ref;

                let get_num = |key: &str| -> Option<i32> {
                    let cf_key = CFString::from_str(key);
                    let val = dict.value(cf_key.as_ref() as *const CFString as *const c_void);
                    if val.is_null() {
                        return None;
                    }
                    let mut n: i32 = 0;
                    if (*(val as *const CFNumber)).value(CFNumberType::IntType, &mut n as *mut _ as *mut c_void) {
                        Some(n)
                    } else {
                        None
                    }
                };

                let get_str = |key: &str| -> Option<String> {
                    let cf_key = CFString::from_str(key);
                    let val = dict.value(cf_key.as_ref() as *const CFString as *const c_void);
                    if val.is_null() {
                        return None;
                    }
                    Some((*(val as *const CFString)).to_string())
                };

                // 提取窗口 handle (kCGWindowNumber)
                let handle = match get_num("kCGWindowNumber") {
                    Some(id) if id > 0 => id as u64,
                    _ => continue,
                };

                // 提取进程 PID
                let pid = get_num("kCGWindowOwnerPID").unwrap_or(0) as u32;
                if pid == 0 || pid == own_pid {
                    continue;
                }

                // 核心过滤：macOS 标准用户级应用窗口层级为 0
                // 剔除 WindowServer (Layer 24/25), Dock (Layer 20), 通知中心 (负数 Layer)
                let layer = get_num("kCGWindowLayer").unwrap_or(0);
                if layer != 0 {
                    continue;
                }

                // 过滤系统全局组件所有者
                let owner_name = get_str("kCGWindowOwnerName").unwrap_or_default();
                if owner_name == "Window Server"
                    || owner_name == "Dock"
                    || owner_name == "ControlCenter"
                    || owner_name == "NotificationCenter"
                    || owner_name == "loginwindow"
                    || owner_name == "Spotlight"
                {
                    continue;
                }

                // 解析窗口真实矩形尺寸
                let mut rect = CGRect::ZERO;
                let bounds_key = CFString::from_str("kCGWindowBounds");
                let bounds_val = dict.value(bounds_key.as_ref() as *const CFString as *const c_void);
                if !bounds_val.is_null() {
                    CGRectMakeWithDictionaryRepresentation(
                        Some(&*(bounds_val as *const CFDictionary)),
                        &mut rect,
                    );
                }
                let width = rect.size.width as u32;
                let height = rect.size.height as u32;
                if width < 32 || height < 32 {
                    continue;
                }

                // 通过 NSRunningApplication 获取系统准确本地化应用名称
                let running_app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid as i32);
                let app_name = running_app
                    .as_ref()
                    .and_then(|a| a.localizedName())
                    .map(|s| s.to_string())
                    .unwrap_or(owner_name);

                let raw_title = get_str("kCGWindowName").unwrap_or_default();
                let title = if !raw_title.trim().is_empty() {
                    raw_title
                } else {
                    format!("{} 窗口", app_name)
                };

                result.push(ExternalWindowInfo {
                    handle,
                    title,
                    process_name: app_name,
                    process_id: pid,
                    x: rect.origin.x as i32,
                    y: rect.origin.y as i32,
                    width,
                    height,
                    is_topmost: false,
                });
            }
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
            use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
            // 优先直接调用 NSRunningApplication 激活该应用至最前台
            if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(info.process_id as i32) {
                #[allow(deprecated)]
                let _ = app.activateWithOptions(NSApplicationActivationOptions::ActivateIgnoringOtherApps);
                return Ok(());
            }
            // 回退到 LaunchServices open 命令
            let status = Command::new("/usr/bin/open")
                .args(["-a", &info.process_name])
                .status()
                .map_err(native_error)?;
            if !status.success() {
                return Err(HalError::NativeCallFailed(format!(
                    "无法激活外部应用程序 '{}'",
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

    /// 激活目标真实外部窗口（用于点击交互）
    pub fn activate_external_window(&self, handle: WindowHandleId) -> HalResult<()> {
        if !self.native_enabled {
            return self.fallback.set_external_window_topmost(handle, true);
        }
        self.activate_native_window(handle)
    }

    #[cfg(target_os = "macos")]
    /// 利用 macOS 原生 Accessibility (AXUIElement) 执行 `AXRaise` 动作：
    /// 将目标物理窗口无感提升至屏幕最顶层，不剥夺当前前台操作焦点，且开销几乎为 0% CPU。
    pub fn raise_native_window(&self, handle: WindowHandleId) -> HalResult<()> {
        use std::ffi::c_void;

        #[link(name = "ApplicationServices", kind = "framework")]
        extern "C" {
            fn AXUIElementCreateApplication(pid: i32) -> *mut c_void;
            fn AXUIElementCopyAttributeValue(
                element: *mut c_void,
                attribute: *const c_void,
                value: *mut *mut c_void,
            ) -> i32;
            fn AXUIElementPerformAction(element: *mut c_void, action: *const c_void) -> i32;
            fn CFRelease(cf: *const c_void);
        }

        use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
        use objc2_core_foundation::{CFArray, CFString};

        let info = self.find_native_window(handle)?;

        unsafe {
            let app_ax = AXUIElementCreateApplication(info.process_id as i32);
            if !app_ax.is_null() {
                let attr_windows = CFString::from_str("AXWindows");
                let mut windows_val: *mut c_void = std::ptr::null_mut();
                let err = AXUIElementCopyAttributeValue(
                    app_ax,
                    attr_windows.as_ref() as *const CFString as *const c_void,
                    &mut windows_val,
                );

                if err == 0 && !windows_val.is_null() {
                    let windows_array = windows_val as *const CFArray;
                    let count = (*windows_array).count();
                    let action_raise = CFString::from_str("AXRaise");

                    for i in 0..count {
                        let win_ax = (*windows_array).value_at_index(i);
                        if !win_ax.is_null() {
                            AXUIElementPerformAction(
                                win_ax as *mut c_void,
                                action_raise.as_ref() as *const CFString as *const c_void,
                            );
                        }
                    }
                    CFRelease(windows_val);
                    CFRelease(app_ax);
                    return Ok(());
                }
                CFRelease(app_ax);
            }
        }

        // 降级回退：若尚未获得系统辅助功能权限，通过 NSRunningApplication 激活拉起目标应用窗口
        if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(info.process_id as i32) {
            #[allow(deprecated)]
            let _ = app.activateWithOptions(NSApplicationActivationOptions::ActivateIgnoringOtherApps);
        }

        Ok(())
    }

    /// 捕获目标外部窗口的真实画面并编码为 JPEG base64 DataURL
    pub fn capture_window_data_url(&self, handle: WindowHandleId) -> HalResult<String> {
        use base64::{engine::general_purpose::STANDARD, Engine as _};

        if !self.native_enabled {
            return Err(HalError::Unsupported("mock_window_capture"));
        }

        #[cfg(target_os = "macos")]
        {
            use objc2_core_foundation::CGRect;
            #[allow(deprecated)]
            use objc2_core_graphics::{
                CGDataProvider, CGImage, CGWindowImageOption, CGWindowListCreateImage,
                CGWindowListOption,
            };

            #[allow(deprecated)]
            let cg_image = CGWindowListCreateImage(
                CGRect::ZERO,
                CGWindowListOption::OptionIncludingWindow,
                handle as u32,
                CGWindowImageOption::Default,
            )
            .ok_or_else(|| HalError::NotFound(format!("外部窗口 {} 截屏失败", handle)))?;

                let width = CGImage::width(Some(&*cg_image));
                let height = CGImage::height(Some(&*cg_image));
                if width == 0 || height == 0 {
                    return Err(HalError::NativeCallFailed("截取到的窗口尺寸为 0".to_string()));
                }

                let data_provider = CGImage::data_provider(Some(&*cg_image))
                    .ok_or_else(|| HalError::NativeCallFailed("获取 CGDataProvider 失败".to_string()))?;
                let data = CGDataProvider::data(Some(&*data_provider))
                    .ok_or_else(|| HalError::NativeCallFailed("复制图像像素数据失败".to_string()))?
                    .to_vec();

                let bytes_per_row = CGImage::bytes_per_row(Some(&*cg_image));
                let mut buffer = Vec::with_capacity(width * height * 4);
                for row in data.chunks_exact(bytes_per_row) {
                    if row.len() >= width * 4 {
                        buffer.extend_from_slice(&row[..width * 4]);
                    }
                }

                for bgra in buffer.chunks_exact_mut(4) {
                    bgra.swap(0, 2);
                }

                let dynamic = image::DynamicImage::ImageRgba8(
                    image::RgbaImage::from_raw(width as u32, height as u32, buffer)
                        .ok_or_else(|| HalError::NativeCallFailed("构建 RGBA 图像失败".to_string()))?,
                );
                let resized = if dynamic.width() > 960 {
                    dynamic.resize(960, u32::MAX, image::imageops::FilterType::Triangle)
                } else {
                    dynamic
                };
                let mut encoded = Vec::new();
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 72)
                    .encode_image(&resized)
                    .map_err(native_error)?;
                return Ok(format!(
                    "data:image/jpeg;base64,{}",
                    STANDARD.encode(encoded)
                ));
        }

        #[cfg(not(target_os = "macos"))]
        {
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
        #[cfg(target_os = "macos")]
        {
            use objc2_app_kit::NSWorkspace;
            let own_pid = std::process::id();
            let workspace = NSWorkspace::sharedWorkspace();
            if let Some(front_app) = workspace.frontmostApplication() {
                let front_pid = front_app.processIdentifier() as u32;
                if front_pid != own_pid && front_pid > 0 {
                    let candidates = self.native_windows()?;
                    if let Some(win) = candidates.into_iter().find(|w| w.process_id == front_pid) {
                        return Ok(win);
                    }
                }
            }
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
            use std::os::raw::{c_int, c_uint};
            #[link(name = "CoreGraphics", kind = "framework")]
            extern "C" {
                fn CGSMainConnectionID() -> c_int;
                fn CGSSetWindowLevel(cid: c_int, wid: c_uint, level: c_int) -> c_int;
            }

            // macOS CoreGraphics / SkyLight 窗口渲染层级规范：
            // kCGNormalWindowLevel = 0: 普通应用默认层级
            // kCGFloatingWindowLevel = 3: 浮动窗口层级（WindowServer 将其物理渲染在所有普通应用之上，点击其他应用不会被遮挡）
            let level: c_int = if topmost { 3 } else { 0 };
            unsafe {
                let cid = CGSMainConnectionID();
                let err = CGSSetWindowLevel(cid, handle as c_uint, level);
                if err != 0 {
                    tracing::warn!(handle, level, err, "CGSSetWindowLevel 返回非零状态码");
                }
            }
            if topmost {
                let _ = self.raise_native_window(handle);
            }
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

    #[test]
    fn test_native_windows_list() {
        let hal = NativeOsHal::new();
        let windows = hal.list_external_windows().unwrap();
        // 校验：必须绝不包含 Window Server、Dock 等系统合成服务
        for win in &windows {
            assert_ne!(win.process_name, "Window Server", "严禁将 Window Server 列入普通应用候选窗口");
            assert_ne!(win.process_name, "Dock", "严禁将 Dock 列入普通应用候选窗口");
            assert!(win.width >= 32, "窗口宽度必须合规");
            assert!(win.height >= 32, "窗口高度必须合规");
            assert!(win.handle > 0, "窗口句柄必须合法有效");
        }
    }

    #[test]
    fn test_cgs_set_window_level() {
        #[cfg(target_os = "macos")]
        {
            use std::os::raw::{c_int, c_uint};
            type CGSConnectionID = c_int;
            type CGSWindowID = c_uint;
            type CGWindowLevel = c_int;
            type CGError = c_int;

            #[link(name = "CoreGraphics", kind = "framework")]
            extern "C" {
                fn CGSMainConnectionID() -> CGSConnectionID;
                fn CGSSetWindowLevel(cid: CGSConnectionID, wid: CGSWindowID, level: CGWindowLevel) -> CGError;
            }

            let hal = NativeOsHal::new();
            let windows = hal.list_external_windows().unwrap();
            if let Some(target) = windows.first() {
                println!("Testing CGSSetWindowLevel on window id={}, app={}", target.handle, target.process_name);
                unsafe {
                    let cid = CGSMainConnectionID();
                    let err_pin = CGSSetWindowLevel(cid, target.handle as u32, 3);
                    println!("CGSSetWindowLevel(..., 3) err = {}", err_pin);
                    assert_eq!(err_pin, 0);

                    // 还原
                    let err_unpin = CGSSetWindowLevel(cid, target.handle as u32, 0);
                    println!("CGSSetWindowLevel(..., 0) err = {}", err_unpin);
                    assert_eq!(err_unpin, 0);

                    // 验证 NativeOsHal::set_external_window_topmost 集成调用
                    assert!(hal.set_external_window_topmost(target.handle, true).is_ok());
                    assert!(hal.set_external_window_topmost(target.handle, false).is_ok());
                }
            }
        }
    }
}
