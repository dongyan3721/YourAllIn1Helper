//! 截图模块对系统剪贴板的最小依赖端口。
//!
//! 模块不直接依赖剪贴板库：生产宿主注入真实系统剪贴板实现，自动化测试注入显式的记录实现。
//! 未注入时使用 [`UnavailableClipboard`]，复制操作如实失败而不是假装成功。

/// 写入系统剪贴板的能力。
pub trait ScreenshotClipboard: Send + Sync + 'static {
    /// 写入 RGBA8 图像（`rgba.len() == width * height * 4`）。
    fn write_image(&self, width: u32, height: u32, rgba: &[u8]) -> Result<(), String>;
    /// 写入纯文本（取色结果）。
    fn write_text(&self, text: &str) -> Result<(), String>;
}

/// 宿主未提供剪贴板时的实现：所有写入返回明确错误。
#[derive(Debug, Default, Clone, Copy)]
pub struct UnavailableClipboard;

impl ScreenshotClipboard for UnavailableClipboard {
    fn write_image(&self, _width: u32, _height: u32, _rgba: &[u8]) -> Result<(), String> {
        Err("当前运行环境未接入系统剪贴板".to_string())
    }

    fn write_text(&self, _text: &str) -> Result<(), String> {
        Err("当前运行环境未接入系统剪贴板".to_string())
    }
}
