//! 截图模块的真实系统剪贴板适配器，仅由 Tauri 生产启动流程注入。
use mod_screenshot::clipboard::ScreenshotClipboard;
#[cfg(not(target_os = "windows"))]
use std::borrow::Cow;

pub(crate) struct ArboardScreenshotClipboard;

impl ScreenshotClipboard for ArboardScreenshotClipboard {
    fn write_image(&self, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
        #[cfg(target_os = "windows")]
        {
            // 与屏幕注释共用带所有者窗口的写入，避免全屏位图触发 1418。
            return os_hal::write_rgba_clipboard(width, height, rgba);
        }
        #[cfg(not(target_os = "windows"))]
        {
            let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
            clipboard
                .set_image(arboard::ImageData {
                    width: width as usize,
                    height: height as usize,
                    bytes: Cow::Borrowed(rgba),
                })
                .map_err(|e| e.to_string())
        }
    }

    fn write_text(&self, text: &str) -> Result<(), String> {
        let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
        clipboard
            .set_text(text.to_string())
            .map_err(|e| e.to_string())
    }

    fn write_gif(&self, bytes: &[u8]) -> Result<(), String> {
        os_hal::write_animated_gif_clipboard(bytes)
    }
}
