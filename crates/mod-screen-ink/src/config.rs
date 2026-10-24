//! 桌面屏幕注释（Screen Ink）模块配置结构体。
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenInkConfig {
    /// 默认初始绘图工具（pen, highlighter, laser, line, rect, arrow, eraser）
    pub default_tool: String,
    /// 默认墨迹颜色（十六进制颜色值，如 #ef4444）
    pub default_color: String,
    /// 默认线条宽度（像素，1.0 ~ 50.0）
    pub default_width: f64,
    /// 截图快照自定义保存目录（为 None 时保存至应用数据目录下的 snapshots 文件夹）
    pub save_directory: Option<String>,
    /// 快照保存时是否自动复制到系统剪贴板
    pub copy_to_clipboard_on_save: bool,
    /// 是否锁定为主屏幕全屏画布（支持 macOS HiDPI 缩放与多屏主显示器识别）
    pub primary_monitor_only: bool,
}

impl Default for ScreenInkConfig {
    fn default() -> Self {
        Self {
            default_tool: "pen".to_string(),
            default_color: "#ef4444".to_string(),
            default_width: 4.0,
            save_directory: None,
            copy_to_clipboard_on_save: true,
            primary_monitor_only: true,
        }
    }
}

impl ScreenInkConfig {
    /// 校验并规范化配置值，避免非法工具类型或极端数值导致渲染异常
    pub fn sanitize(&mut self) {
        let valid_tools = [
            "pen",
            "highlighter",
            "laser",
            "line",
            "rect",
            "arrow",
            "eraser",
        ];
        if !valid_tools.contains(&self.default_tool.as_str()) {
            self.default_tool = "pen".to_string();
        }

        let hex_valid = self.default_color.starts_with('#')
            && (self.default_color.len() == 7 || self.default_color.len() == 4)
            && self.default_color[1..]
                .chars()
                .all(|c| c.is_ascii_hexdigit());
        if !hex_valid {
            self.default_color = "#ef4444".to_string();
        }

        if self.default_width <= 0.5 || self.default_width > 50.0 || self.default_width.is_nan() {
            self.default_width = 4.0;
        }

        if let Some(ref dir) = self.save_directory {
            if dir.trim().is_empty() {
                self.save_directory = None;
            }
        }
    }
}
