//! 显示器截屏及外部窗口画面编码；不包含业务模块逻辑。
use super::{native_error, NativeOsHal};
use crate::{
    CapturedFrame, DisplayMonitorInfo, HalError, HalResult, ScreenCapturer, WindowHandleId,
};
use app_core::SelectionBox;
use xcap::Monitor;
#[cfg(not(target_os = "macos"))]
use xcap::Window;

impl ScreenCapturer for NativeOsHal {
    fn list_monitors(&self) -> HalResult<Vec<DisplayMonitorInfo>> {
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

    #[cfg(target_os = "macos")]
    fn check_capture_permission(&self) -> HalResult<()> {
        use objc2_core_graphics::CGPreflightScreenCaptureAccess;
        // 仅预检，不调用 CGRequestScreenCaptureAccess，避免未经用户操作弹出系统授权。
        // 独立启动的程序上，预检接口会在权限已经生效时仍返回 false；
        // 这时其他应用的普通窗口标题已经能读到，按实际能力放行。
        if CGPreflightScreenCaptureAccess() || macos_screen_recording_titles_visible() {
            Ok(())
        } else {
            let path = std::env::current_exe()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|_| "all-in-one-desktop".to_string());
            Err(HalError::PermissionDenied {
                capability: "screen_recording",
                guidance: format!(
                    "请在「系统设置 → 隐私与安全性 → 屏幕与系统录音」中允许这个程序：{path}。允许后必须完全退出并重新打开。终端、Cursor 或旧的测试 App 上的开关不会作用到这个程序"
                ),
            })
        }
    }
}

/// 无录屏权限时，其他应用的普通窗口没有标题，只剩 Window Server 等系统窗口。
/// 能读到其他应用的标题，说明录屏权限已经生效。
#[cfg(target_os = "macos")]
fn macos_screen_recording_titles_visible() -> bool {
    use objc2_core_foundation::{CFDictionary, CFNumber, CFNumberType, CFString};
    use objc2_core_graphics::{CGWindowListCopyWindowInfo, CGWindowListOption};
    use std::ffi::c_void;

    let own_pid = std::process::id() as i32;
    unsafe {
        let Some(windows) = CGWindowListCopyWindowInfo(
            CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements,
            0,
        ) else {
            return false;
        };
        for index in 0..windows.count() {
            let dictionary = windows.value_at_index(index) as *const CFDictionary;
            if dictionary.is_null() {
                continue;
            }
            let dictionary = &*dictionary;
            let read_number = |key: &str| -> Option<i32> {
                let cf_key = CFString::from_str(key);
                let value = dictionary.value(cf_key.as_ref() as *const CFString as *const c_void);
                if value.is_null() {
                    return None;
                }
                let mut number = 0;
                (*(value as *const CFNumber))
                    .value(CFNumberType::IntType, &mut number as *mut _ as *mut c_void)
                    .then_some(number)
            };
            let read_string = |key: &str| -> Option<String> {
                let cf_key = CFString::from_str(key);
                let value = dictionary.value(cf_key.as_ref() as *const CFString as *const c_void);
                if value.is_null() {
                    return None;
                }
                Some((*(value as *const CFString)).to_string())
            };
            if read_number("kCGWindowOwnerPID") == Some(own_pid) {
                continue;
            }
            // 普通应用窗口在第 0 层。菜单栏、桌面和程序坞不拿来当授权证据。
            if read_number("kCGWindowLayer").unwrap_or(0) != 0 {
                continue;
            }
            let owner = read_string("kCGWindowOwnerName").unwrap_or_default();
            if owner == "Window Server"
                || owner == "Dock"
                || owner == "ControlCenter"
                || owner == "NotificationCenter"
                || owner == "loginwindow"
                || owner == "Spotlight"
            {
                continue;
            }
            if read_string("kCGWindowName").is_some_and(|title| !title.trim().is_empty()) {
                return true;
            }
        }
    }
    false
}

impl NativeOsHal {
    pub(super) fn capture_native_window_data_url(
        &self,
        handle: WindowHandleId,
    ) -> HalResult<String> {
        use base64::{engine::general_purpose::STANDARD, Engine as _};

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
                return Err(HalError::NativeCallFailed(
                    "截取到的窗口尺寸为 0".to_string(),
                ));
            }

            let data_provider = CGImage::data_provider(Some(&*cg_image)).ok_or_else(|| {
                HalError::NativeCallFailed("获取 CGDataProvider 失败".to_string())
            })?;
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
}
