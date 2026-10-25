//! 划词翻译配置定义与参数清洗。
//!
//! 关键约束：
//! - 任何敏感密钥（API Key）严禁包含在此结构中，避免序列化写入 config.json。
//! - 默认关闭历史记录与自动外发，尊重用户隐私。
//! - 支持三语言首选目标语（中/英/日）与自定义 OpenAI 兼容接口。

use serde::{Deserialize, Serialize};

/// 允许的核心语言标识。
pub const SUPPORTED_LANGUAGES: &[&str] = &[
    "auto",  // 自动检测
    "zh-CN", // 简体中文
    "en-US", // 英语
    "ja-JP", // 日语
];

/// 自定义翻译服务源配置（非敏感元数据）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomProviderConfig {
    pub id: String,
    pub name: String,
    pub endpoint_url: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default = "default_true")]
    pub is_openai_compatible: bool,
}

fn default_true() -> bool {
    true
}

/// 划词翻译全局配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranslatorConfig {
    /// 默认启用的翻译源（"google" | "deepl" | "openai" 或自定义 ID）
    #[serde(default = "default_provider")]
    pub default_provider_id: String,

    /// 目标语言代码（默认 "zh-CN"）
    #[serde(default = "default_target_lang")]
    pub target_lang: String,

    /// 源语言代码（默认 "auto"）
    #[serde(default = "default_source_lang")]
    pub source_lang: String,

    /// 翻译完成后是否自动写入剪贴板（默认关闭）
    #[serde(default)]
    pub auto_copy_result: bool,

    /// 是否保留本地翻译历史记录（默认关闭，选区不擅自留存）
    #[serde(default)]
    pub history_enabled: bool,

    /// 全局快捷键（默认 PrimaryMod+Shift+T）
    #[serde(default = "default_hotkey")]
    pub hotkey: String,

    /// 浮动气泡窗口宽度
    #[serde(default = "default_popover_width")]
    pub popover_width: f64,

    /// 浮动气泡窗口高度
    #[serde(default = "default_popover_height")]
    pub popover_height: f64,

    /// 用户自定义服务提供商列表
    #[serde(default)]
    pub custom_providers: Vec<CustomProviderConfig>,
}

fn default_provider() -> String {
    "google".to_string()
}

fn default_target_lang() -> String {
    "zh-CN".to_string()
}

fn default_source_lang() -> String {
    "auto".to_string()
}

fn default_hotkey() -> String {
    "PrimaryMod+Shift+T".to_string()
}

fn default_popover_width() -> f64 {
    420.0
}

fn default_popover_height() -> f64 {
    340.0
}

impl Default for TranslatorConfig {
    fn default() -> Self {
        Self {
            default_provider_id: default_provider(),
            target_lang: default_target_lang(),
            source_lang: default_source_lang(),
            auto_copy_result: false,
            history_enabled: false,
            hotkey: default_hotkey(),
            popover_width: default_popover_width(),
            popover_height: default_popover_height(),
            custom_providers: Vec::new(),
        }
    }
}

impl TranslatorConfig {
    /// 校验并容错清洗配置参数。
    pub fn sanitize(&mut self) {
        if self.default_provider_id.trim().is_empty() {
            self.default_provider_id = default_provider();
        }

        let valid_targets = ["zh-CN", "en-US", "ja-JP", "zh", "en", "ja"];
        if !valid_targets.contains(&self.target_lang.as_str()) {
            self.target_lang = default_target_lang();
        }

        if self.source_lang.trim().is_empty() {
            self.source_lang = default_source_lang();
        }

        if self.hotkey.trim().is_empty() {
            self.hotkey = default_hotkey();
        }

        if self.popover_width < 320.0 || self.popover_width > 1200.0 {
            self.popover_width = default_popover_width();
        }

        if self.popover_height < 200.0 || self.popover_height > 900.0 {
            self.popover_height = default_popover_height();
        }

        // 清洗自定义 provider
        self.custom_providers.retain(|p| {
            !p.id.trim().is_empty()
                && !p.name.trim().is_empty()
                && (p.endpoint_url.starts_with("http://") || p.endpoint_url.starts_with("https://"))
        });
    }
}
