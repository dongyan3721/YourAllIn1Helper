//! 模块 01 自有配置，存放在 `config.json` 的 `modules.settings.screenshot`。
//!
//! 智能选区、放大镜等开关沿用核心层已有的 `AppConfig.screenshot`（旧版类型化配置），
//! 本结构只保存 M1 新增的保存、标注与贴图偏好，未知字段原样保留以便后续版本升级。
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// 标注默认颜色（与前端调色板第一项一致）。
pub const DEFAULT_ANNOTATION_COLOR: &str = "#ff3b30";
/// 同时存在的贴图数量上限，防止误操作创建大量常驻 WebView。
pub const MAX_PINS_LIMIT: usize = 64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenshotSettings {
    /// 保存目录；为空时使用应用数据目录下的 `screenshots/`。
    pub save_directory: Option<String>,
    /// 保存文件名前缀，最终文件名为 `{prefix}_{YYYYmmdd_HHMMSS}.png`。
    pub file_name_prefix: String,
    /// 保存成功后是否同时复制到系统剪贴板。
    pub copy_after_save: bool,
    /// 标注默认颜色（#RRGGBB）。
    pub default_color: String,
    /// 标注默认线宽（逻辑像素，前端按设备像素比换算）。
    pub default_stroke_width: f64,
    /// 新建贴图的默认不透明度（0.1–1.0）。
    pub default_pin_opacity: f64,
    /// 同时存在的贴图数量上限。
    pub max_pins: usize,
    /// 新版本写入、当前版本不认识的字段，保存时原样写回。
    #[serde(flatten)]
    pub unknown_fields: Map<String, Value>,
}

impl Default for ScreenshotSettings {
    fn default() -> Self {
        Self {
            save_directory: None,
            file_name_prefix: "screenshot".to_string(),
            copy_after_save: false,
            default_color: DEFAULT_ANNOTATION_COLOR.to_string(),
            default_stroke_width: 4.0,
            default_pin_opacity: 1.0,
            max_pins: 16,
            unknown_fields: Map::new(),
        }
    }
}

impl ScreenshotSettings {
    /// 规范化非法值；只修正本结构认识的字段，不触碰未知字段。
    pub fn sanitize(&mut self) {
        if let Some(dir) = &self.save_directory {
            if dir.trim().is_empty() {
                self.save_directory = None;
            }
        }
        // 前缀只允许常见文件名字符，避免路径分隔符把文件写到保存目录之外。
        let cleaned: String = self
            .file_name_prefix
            .trim()
            .chars()
            .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | ' '))
            .take(48)
            .collect();
        self.file_name_prefix = if cleaned.trim().is_empty() {
            "screenshot".to_string()
        } else {
            cleaned.trim().to_string()
        };
        if !is_hex_color(&self.default_color) {
            self.default_color = DEFAULT_ANNOTATION_COLOR.to_string();
        }
        if !self.default_stroke_width.is_finite()
            || self.default_stroke_width < 1.0
            || self.default_stroke_width > 32.0
        {
            self.default_stroke_width = 4.0;
        }
        if !self.default_pin_opacity.is_finite() {
            self.default_pin_opacity = 1.0;
        }
        self.default_pin_opacity = self.default_pin_opacity.clamp(0.1, 1.0);
        self.max_pins = self.max_pins.clamp(1, MAX_PINS_LIMIT);
    }
}

/// 把前端提交的部分字段合并进现有配置并规范化；未出现的字段与未知字段保持不变。
pub fn merge_settings(
    current: &ScreenshotSettings,
    patch: &Value,
) -> Result<ScreenshotSettings, String> {
    let patch = patch
        .as_object()
        .ok_or_else(|| "截图设置必须是 JSON 对象".to_string())?;
    let mut merged = serde_json::to_value(current).map_err(|e| e.to_string())?;
    let target = merged
        .as_object_mut()
        .ok_or_else(|| "截图设置序列化异常".to_string())?;
    for (key, value) in patch {
        target.insert(key.clone(), value.clone());
    }
    let mut settings: ScreenshotSettings =
        serde_json::from_value(merged).map_err(|e| format!("截图设置字段类型错误：{}", e))?;
    settings.sanitize();
    Ok(settings)
}

/// 校验 `#RRGGBB` 颜色格式。
pub fn is_hex_color(value: &str) -> bool {
    value.len() == 7 && value.starts_with('#') && value[1..].chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
#[path = "../tests/unit/config.rs"]
mod tests;
