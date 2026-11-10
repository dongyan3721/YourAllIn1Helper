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
        if bounds.width == 0 || bounds.height == 0 {
            return Err(HalError::NativeCallFailed("抓取区域尺寸为 0".to_string()));
        }
        let monitors = Monitor::all().map_err(native_error)?;
        let monitor = monitors
            .into_iter()
            .find(|monitor| {
                let Ok(x) = monitor.x() else { return false };
                let Ok(y) = monitor.y() else { return false };
                let Ok(w) = monitor.width() else { return false };
                let Ok(h) = monitor.height() else {
                    return false;
                };
                bounds.x >= x
                    && bounds.y >= y
                    && (bounds.x as i64) < x as i64 + w as i64
                    && (bounds.y as i64) < y as i64 + h as i64
            })
            .ok_or_else(|| HalError::NotFound("选区不在任何显示器内".to_string()))?;
        let origin_x = monitor.x().map_err(native_error)?;
        let origin_y = monitor.y().map_err(native_error)?;
        let monitor_w = monitor.width().map_err(native_error)?;
        let monitor_h = monitor.height().map_err(native_error)?;
        let rel_x = (bounds.x - origin_x).max(0) as u32;
        let rel_y = (bounds.y - origin_y).max(0) as u32;
        let width = bounds.width.min(monitor_w.saturating_sub(rel_x)).max(1);
        let height = bounds.height.min(monitor_h.saturating_sub(rel_y)).max(1);
        let image = monitor
            .capture_region(rel_x, rel_y, width, height)
            .map_err(native_error)?;
        Ok(CapturedFrame {
            width: image.width(),
            height: image.height(),
            scale_factor_milli: (monitor.scale_factor().unwrap_or(1.0) * 1000.0) as u32,
            rgba_bytes: image.into_raw(),
        })
    }

    fn capture_region_below(
        &self,
        bounds: SelectionBox,
        below_window_id: u32,
    ) -> HalResult<CapturedFrame> {
        // Windows 的遮罩句柄是 64 位 HWND，旧 u32 参数无法承载；此时复用已登记的
        // 原生排除窗口，长截图无需另写平台分支即可获得同样的遮罩排除。
        let exclusion = resolve_capture_exclusion(below_window_id, self.active_capture_exclusion());
        self.capture_region_excluding(bounds, exclusion)
    }

    fn set_capture_exclusion(&self, handle: Option<WindowHandleId>) -> HalResult<()> {
        let next = handle.filter(|window| *window != 0);
        // 登记与恢复由同一把锁串行化，替换窗口时不会留下旧的捕获排除。
        let mut registered = self.capture_exclusion.lock();
        #[cfg(target_os = "windows")]
        {
            if let Some(window) = next {
                // 登记本身不隐藏普通选区和结果预览；真正采集前由 prepare_live_capture 启用。
                hwnd_from_window_handle(window)?;
            }
            if let Some(previous) = registered.filter(|previous| Some(*previous) != next) {
                // 旧窗口可能已销毁；不让失效句柄妨碍会话收尾。
                let _ = set_windows_capture_exclusion(previous, false);
            }
        }
        *registered = next;
        Ok(())
    }

    fn active_capture_exclusion(&self) -> Option<WindowHandleId> {
        *self.capture_exclusion.lock()
    }

    fn prepare_live_capture(&self) -> HalResult<()> {
        #[cfg(target_os = "windows")]
        {
            let window = self.active_capture_exclusion().ok_or_else(|| {
                HalError::NativeCallFailed(
                    "Windows 实时捕获的遮罩排除未就绪，请取消截图后重试".to_string(),
                )
            })?;
            set_windows_capture_exclusion(window, true)?;
        }
        Ok(())
    }

    fn finish_live_capture(&self) -> HalResult<()> {
        #[cfg(target_os = "windows")]
        if let Some(window) = self.active_capture_exclusion() {
            set_windows_capture_exclusion(window, false)?;
        }
        Ok(())
    }

    fn capture_region_excluding(
        &self,
        bounds: SelectionBox,
        exclusion: Option<WindowHandleId>,
    ) -> HalResult<CapturedFrame> {
        #[cfg(target_os = "macos")]
        {
            if let Some(window) = exclusion {
                // CGWindowID 是 32 位；超范围说明调用方传入了错误平台的 HWND。
                let window_id = u32::try_from(window).map_err(|_| {
                    HalError::NativeCallFailed(format!("CGWindowID 超出 32 位：{window}"))
                })?;
                return capture_below_window(bounds, window_id);
            }
            return self.capture_region(bounds);
        }
        #[cfg(not(target_os = "macos"))]
        {
            #[cfg(target_os = "windows")]
            if let Some(window) = exclusion {
                // 注册时已设置；每帧重申一次，避免遮罩被其他代码重置后仍录到黑帧。
                set_windows_capture_exclusion(window, true)?;
            }
            let _ = exclusion;
            self.capture_region(bounds)
        }
    }

    #[cfg(target_os = "macos")]
    fn check_capture_permission(&self) -> HalResult<()> {
        super::screen_permission::check()
    }

    #[cfg(target_os = "macos")]
    fn request_capture_permission(&self) -> HalResult<()> {
        super::screen_permission::request()
    }
}

/// 旧 `u32` 遮罩号优先；为 0 时退回已登记的原生排除句柄（Windows 64 位 HWND）。
fn resolve_capture_exclusion(
    below_window_id: u32,
    registered: Option<WindowHandleId>,
) -> Option<WindowHandleId> {
    if below_window_id == 0 {
        registered
    } else {
        Some(below_window_id as WindowHandleId)
    }
}

#[cfg(target_os = "windows")]
fn hwnd_from_window_handle(handle: WindowHandleId) -> HalResult<windows::Win32::Foundation::HWND> {
    if handle == 0 {
        return Err(HalError::NativeCallFailed("遮罩窗口句柄为 0".to_string()));
    }
    let raw = usize::try_from(handle)
        .map_err(|_| HalError::NativeCallFailed(format!("HWND 超出当前进程指针宽度：{handle}")))?;
    Ok(windows::Win32::Foundation::HWND(
        raw as *mut core::ffi::c_void,
    ))
}

/// 让遮罩窗口从抓屏结果中彻底消失，露出底下真实画面。
#[cfg(target_os = "windows")]
fn set_windows_capture_exclusion(handle: WindowHandleId, exclude: bool) -> HalResult<()> {
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
    };

    let hwnd = hwnd_from_window_handle(handle)?;
    let affinity = if exclude {
        WDA_EXCLUDEFROMCAPTURE
    } else {
        WDA_NONE
    };
    // SAFETY: 句柄由 Tauri 拥有且属于当前进程；WDA 只影响抓屏，不改变窗口可见性。
    unsafe { SetWindowDisplayAffinity(hwnd, affinity) }.map_err(native_error)
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

/// 只抓指定窗口之下的全局桌面矩形，避免 GIF / 长截图把遮罩自己拍进去。
#[cfg(target_os = "macos")]
fn capture_below_window(bounds: SelectionBox, below_window_id: u32) -> HalResult<CapturedFrame> {
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    #[allow(deprecated)]
    use objc2_core_graphics::{
        CGDataProvider, CGImage, CGWindowImageOption, CGWindowListCreateImage, CGWindowListOption,
    };

    if bounds.width == 0 || bounds.height == 0 {
        return Err(HalError::NativeCallFailed("抓取区域尺寸为 0".to_string()));
    }
    let rect = CGRect {
        origin: CGPoint {
            x: bounds.x as f64,
            y: bounds.y as f64,
        },
        size: CGSize {
            width: bounds.width as f64,
            height: bounds.height as f64,
        },
    };
    #[allow(deprecated)]
    let cg_image = CGWindowListCreateImage(
        rect,
        CGWindowListOption::OptionOnScreenBelowWindow,
        below_window_id,
        CGWindowImageOption::Default,
    )
    .ok_or_else(|| HalError::NativeCallFailed("采集窗口之下的画面失败".to_string()))?;
    let width = CGImage::width(Some(&*cg_image));
    let height = CGImage::height(Some(&*cg_image));
    if width == 0 || height == 0 {
        return Err(HalError::NativeCallFailed(
            "采集到的区域尺寸为 0".to_string(),
        ));
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
    let scale_milli = ((width as u64 * 1000) / bounds.width.max(1) as u64) as u32;
    Ok(CapturedFrame {
        width: width as u32,
        height: height as u32,
        scale_factor_milli: scale_milli.max(1),
        rgba_bytes: buffer,
    })
}

#[cfg(test)]
#[path = "../../tests/unit/capture_exclusion.rs"]
mod tests;
