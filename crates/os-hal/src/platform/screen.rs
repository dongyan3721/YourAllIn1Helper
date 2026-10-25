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
                let Ok(h) = monitor.height() else { return false };
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
        if below_window_id == 0 {
            return self.capture_region(bounds);
        }
        #[cfg(target_os = "macos")]
        {
            return capture_below_window(bounds, below_window_id);
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = below_window_id;
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
        return Err(HalError::NativeCallFailed("采集到的区域尺寸为 0".to_string()));
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
