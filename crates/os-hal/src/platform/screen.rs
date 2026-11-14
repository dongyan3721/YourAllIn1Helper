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
                let _ = restore_windows_capture_overlay(previous);
                self.live_capture_region.lock().take();
            }
        }
        *registered = next;
        Ok(())
    }

    fn active_capture_exclusion(&self) -> Option<WindowHandleId> {
        *self.capture_exclusion.lock()
    }

    fn capture_exclusion_alive(&self, handle: WindowHandleId) -> bool {
        if handle == 0 {
            return false;
        }
        #[cfg(target_os = "macos")]
        {
            macos_capture_window_visible(handle)
        }
        #[cfg(target_os = "windows")]
        {
            hwnd_from_window_handle(handle)
                .map(|hwnd| unsafe {
                    windows::Win32::UI::WindowsAndMessaging::IsWindow(hwnd).as_bool()
                })
                .unwrap_or(false)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            true
        }
    }

    fn prepare_live_capture(&self) -> HalResult<()> {
        #[cfg(target_os = "windows")]
        {
            let window = self.active_capture_exclusion().ok_or_else(|| {
                HalError::NativeCallFailed(
                    "Windows 实时捕获的遮罩排除未就绪，请取消截图后重试".to_string(),
                )
            })?;
            hwnd_from_window_handle(window)?;
        }
        Ok(())
    }

    fn prepare_live_capture_region(&self, bounds: SelectionBox) -> HalResult<()> {
        self.prepare_live_capture()?;
        #[cfg(target_os = "windows")]
        {
            let registered = self.capture_exclusion.lock();
            let window = registered.ok_or_else(|| {
                HalError::NativeCallFailed("Windows 实时捕获的遮罩排除未就绪".into())
            })?;
            set_windows_capture_hole(window, bounds)?;
            *self.live_capture_region.lock() = Some((window, bounds));
        }
        let _ = bounds;
        Ok(())
    }

    fn finish_live_capture(&self) -> HalResult<()> {
        #[cfg(target_os = "windows")]
        if let Some(window) = self.active_capture_exclusion() {
            restore_windows_capture_overlay(window)?;
            self.live_capture_region.lock().take();
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
                if *self.live_capture_region.lock() != Some((window, bounds)) {
                    return Err(HalError::NativeCallFailed(
                        "实时抓屏选区尚未从遮罩中移除，请重新开始录制".into(),
                    ));
                }
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

/// 将全局物理像素转换为无边框遮罩的窗口坐标，支持负坐标副屏。
#[cfg(target_os = "windows")]
fn live_hole_rect(
    bounds: SelectionBox,
    window: windows::Win32::Foundation::RECT,
) -> HalResult<windows::Win32::Foundation::RECT> {
    use windows::Win32::Foundation::RECT;
    let left = bounds.x as i64 - window.left as i64;
    let top = bounds.y as i64 - window.top as i64;
    let right = left + bounds.width as i64;
    let bottom = top + bounds.height as i64;
    if bounds.width == 0
        || bounds.height == 0
        || left < 0
        || top < 0
        || right > window.right as i64 - window.left as i64
        || bottom > window.bottom as i64 - window.top as i64
    {
        return Err(HalError::NativeCallFailed(
            "实时抓屏选区超出遮罩窗口范围".into(),
        ));
    }
    Ok(RECT {
        left: left as i32,
        top: top as i32,
        right: right as i32,
        bottom: bottom as i32,
    })
}

#[cfg(target_os = "windows")]
fn restore_windows_capture_overlay(handle: WindowHandleId) -> HalResult<()> {
    use windows::Win32::Graphics::Gdi::SetWindowRgn;
    let hwnd = hwnd_from_window_handle(handle)?;
    unsafe {
        if SetWindowRgn(hwnd, None, true) == 0 {
            return Err(native_error(windows::core::Error::from_thread()));
        }
    }
    Ok(())
}

/// 真正裁掉录制选区，避免 GDI 抓到全屏 DComp/WebView2 遮罩的冻结帧或黑面。
/// 不使用 WDA_EXCLUDEFROMCAPTURE，因此控制 UI 在远程/虚拟显示器上也保持可见。
#[cfg(target_os = "windows")]
fn set_windows_capture_hole(handle: WindowHandleId, bounds: SelectionBox) -> HalResult<()> {
    use windows::Win32::{
        Foundation::RECT,
        Graphics::{
            Dwm::DwmFlush,
            Gdi::{CombineRgn, CreateRectRgn, DeleteObject, SetWindowRgn, RGN_DIFF, RGN_ERROR},
        },
        UI::WindowsAndMessaging::GetWindowRect,
    };
    let hwnd = hwnd_from_window_handle(handle)?;
    unsafe {
        let mut window = RECT::default();
        GetWindowRect(hwnd, &mut window).map_err(native_error)?;
        let hole = live_hole_rect(bounds, window)?;
        let outer = CreateRectRgn(0, 0, window.right - window.left, window.bottom - window.top);
        let inner = CreateRectRgn(hole.left, hole.top, hole.right, hole.bottom);
        if outer.is_invalid() || inner.is_invalid() {
            let _ = DeleteObject(outer.into());
            let _ = DeleteObject(inner.into());
            return Err(native_error(windows::core::Error::from_thread()));
        }
        let combined = CombineRgn(Some(outer), Some(outer), Some(inner), RGN_DIFF);
        let _ = DeleteObject(inner.into());
        if combined == RGN_ERROR || SetWindowRgn(hwnd, Some(outer), true) == 0 {
            let _ = DeleteObject(outer.into());
            return Err(native_error(windows::core::Error::from_thread()));
        }
        // 成功后 outer 归系统所有；等待合成完成，首帧也不应是旧的冻结遮罩。
        let _ = DwmFlush();
    }
    Ok(())
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

/// 判断 macOS 排除窗口（遮罩 CGWindowID）是否仍在屏幕上存活。
/// 用于 GIF 实时录制的 fail-closed 复核：窗口被销毁或重建后不能继续沿用旧号抓屏。
#[cfg(target_os = "macos")]
fn macos_capture_window_visible(handle: WindowHandleId) -> bool {
    use objc2_core_foundation::{CFDictionary, CFNumber, CFNumberType, CFString};
    use objc2_core_graphics::{CGWindowListCopyWindowInfo, CGWindowListOption};
    use std::ffi::c_void;

    let Ok(target) = u32::try_from(handle) else {
        return false;
    };
    if target == 0 {
        return false;
    }
    unsafe {
        let Some(cf_array) = CGWindowListCopyWindowInfo(CGWindowListOption::OptionOnScreenOnly, 0)
        else {
            return false;
        };
        let count = cf_array.count();
        for index in 0..count {
            let dict_ref = cf_array.value_at_index(index) as *const CFDictionary;
            if dict_ref.is_null() {
                continue;
            }
            let dict = &*dict_ref;
            let key = CFString::from_str("kCGWindowNumber");
            let value = dict.value(key.as_ref() as *const CFString as *const c_void);
            if value.is_null() {
                continue;
            }
            let mut number: i32 = 0;
            if (*(value as *const CFNumber))
                .value(CFNumberType::IntType, &mut number as *mut _ as *mut c_void)
                && number > 0
                && number as u32 == target
            {
                return true;
            }
        }
    }
    false
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
