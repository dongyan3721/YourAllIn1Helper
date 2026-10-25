//! 应用使用时长模块的本地配置。
//!
//! 分类与忽略名单属于逐应用的用户设置，存放在 `tracked_apps` 表；
//! 本结构只保存全局采样、刷写和隐私选项，持久化于 `modules.settings.app_usage`。

use serde::{Deserialize, Serialize};

pub const MODULE_ID: &str = "app_usage";
pub const MIN_IDLE_SECS: u64 = 30;
pub const MAX_IDLE_SECS: u64 = 24 * 60 * 60;
pub const MIN_FLUSH_SECS: u64 = 5;
pub const MAX_FLUSH_SECS: u64 = 3600;
pub const VALID_CATEGORIES: &[&str] = &[
    "work",
    "development",
    "communication",
    "browse",
    "media",
    "game",
    "system",
    "other",
];

/// 持久化于 `modules.settings.app_usage` 的全局配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppUsageConfig {
    /// 键鼠无输入达到该秒数后暂停计时。
    pub idle_threshold_secs: u64,
    /// 满该秒数执行一次 SQLite UPSERT；焦点切换和停用仍会立即刷写。
    pub flush_interval_secs: u64,
    /// 是否把窗口标题写入数据库。关闭时仅保留应用标识和名称。
    pub keep_window_titles: bool,
}

impl Default for AppUsageConfig {
    fn default() -> Self {
        Self {
            idle_threshold_secs: 300,
            flush_interval_secs: 60,
            keep_window_titles: true,
        }
    }
}

impl AppUsageConfig {
    /// 夹紧不可信输入，避免 0 秒刷写或极端阈值拖垮采样线程。
    pub fn normalized(mut self) -> Self {
        self.idle_threshold_secs = self.idle_threshold_secs.clamp(MIN_IDLE_SECS, MAX_IDLE_SECS);
        self.flush_interval_secs = self
            .flush_interval_secs
            .clamp(MIN_FLUSH_SECS, MAX_FLUSH_SECS);
        self
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(MIN_IDLE_SECS..=MAX_IDLE_SECS).contains(&self.idle_threshold_secs) {
            return Err(format!(
                "idle_threshold_secs must be between {MIN_IDLE_SECS} and {MAX_IDLE_SECS}"
            ));
        }
        if !(MIN_FLUSH_SECS..=MAX_FLUSH_SECS).contains(&self.flush_interval_secs) {
            return Err(format!(
                "flush_interval_secs must be between {MIN_FLUSH_SECS} and {MAX_FLUSH_SECS}"
            ));
        }
        Ok(())
    }

    pub(crate) fn flush_interval_millis(&self) -> u64 {
        self.flush_interval_secs.saturating_mul(1_000)
    }
}

/// 把外部传入的分类归一化为受支持值，未知值不写入数据库。
pub fn normalize_category(raw: &str) -> Option<String> {
    let value = raw.trim().to_ascii_lowercase();
    VALID_CATEGORIES.contains(&value.as_str()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_normalizes_out_of_range_values() {
        let config = AppUsageConfig {
            idle_threshold_secs: 0,
            flush_interval_secs: 1,
            keep_window_titles: false,
        }
        .normalized();
        assert_eq!(config.idle_threshold_secs, MIN_IDLE_SECS);
        assert_eq!(config.flush_interval_secs, MIN_FLUSH_SECS);
        assert!(!config.keep_window_titles);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn category_accepts_only_known_values() {
        assert_eq!(normalize_category("Work"), Some("work".to_string()));
        assert_eq!(normalize_category("unknown"), None);
        assert_eq!(normalize_category("  "), None);
    }
}
