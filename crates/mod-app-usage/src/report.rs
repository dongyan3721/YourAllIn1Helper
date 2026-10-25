//! `/usage` 仪表盘使用的只读报表结构。
//!
//! 所有时长单位为秒；日期为本地日期（`%Y-%m-%d`），小时为本地 0–23。
//! 夏令时回拨造成的重复本地小时会合并到同一小时桶，春天跳过的小时保持为空。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppUsageSummary {
    pub app_id: String,
    pub app_name: String,
    pub category: String,
    pub ignored: bool,
    pub active_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyUsage {
    pub date: String,
    pub active_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryUsage {
    pub category: String,
    pub active_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageOverview {
    pub start_date: String,
    pub end_date: String,
    pub total_seconds: u64,
    pub days: Vec<DailyUsage>,
    pub apps: Vec<AppUsageSummary>,
    pub categories: Vec<CategoryUsage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HourlyUsage {
    pub hour: u8,
    pub active_seconds: u64,
    pub apps: Vec<AppUsageSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HourlyBreakdown {
    pub date: String,
    pub total_seconds: u64,
    pub hours: Vec<HourlyUsage>,
}

/// `/usage` 面板的逐会话明细；窗口标题只保存于数据库，是否返回由隐私设置决定。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageSession {
    pub id: i64,
    pub app_id: String,
    pub app_name: String,
    pub window_title: Option<String>,
    pub started_at: i64,
    pub ended_at: i64,
    pub active_seconds: u64,
}

/// 会话列表分页结果；`total` 是该日全部会话数，用于上一页/下一页。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageSessionPage {
    pub date: String,
    pub total: u64,
    pub offset: u64,
    pub limit: u64,
    pub sessions: Vec<UsageSession>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackedApp {
    pub app_id: String,
    pub app_name: String,
    pub executable_path: Option<String>,
    pub category: String,
    pub ignored: bool,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
    pub total_seconds: u64,
}

/// 原生能力实际探测结果；未实现的音频 AFK 例外必须显式暴露，不能假装支持。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppUsageCapabilities {
    pub active_app: bool,
    pub idle_detection: bool,
    pub screen_lock_detection: bool,
    pub audio_afk_exception: bool,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppUsageStatus {
    pub tracking: bool,
    pub sample_interval_ms: u64,
    pub capabilities: AppUsageCapabilities,
    pub config: super::config::AppUsageConfig,
}
