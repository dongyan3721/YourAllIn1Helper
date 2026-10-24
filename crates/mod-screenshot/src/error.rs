//! 截图失败的结构化描述：区分权限、不支持、资源缺失与原生调用失败，供主面板给出可操作提示。
use anyhow::anyhow;
use app_core::CoreError;
use os_hal::HalError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    PermissionDenied,
    Unsupported,
    NotFound,
    NativeCallFailed,
    Internal,
}

/// 最近一次失败；`guidance` 只在系统给出明确处理步骤时填写。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenshotFailure {
    pub kind: FailureKind,
    pub capability: Option<String>,
    pub message: String,
    pub guidance: Option<String>,
    pub at: i64,
}

impl ScreenshotFailure {
    pub fn from_hal(error: &HalError) -> Self {
        let at = chrono::Utc::now().timestamp_millis();
        match error {
            HalError::PermissionDenied {
                capability,
                guidance,
            } => Self {
                kind: FailureKind::PermissionDenied,
                capability: Some((*capability).to_string()),
                message: format!("缺少系统权限：{}", capability),
                guidance: Some(guidance.clone()),
                at,
            },
            HalError::Unsupported(capability) => Self {
                kind: FailureKind::Unsupported,
                capability: Some((*capability).to_string()),
                message: format!("当前系统不支持能力：{}", capability),
                guidance: None,
                at,
            },
            HalError::NotFound(detail) => Self {
                kind: FailureKind::NotFound,
                capability: None,
                message: format!("未找到可截取的显示器：{}", detail),
                guidance: None,
                at,
            },
            HalError::NativeCallFailed(detail) => Self {
                kind: FailureKind::NativeCallFailed,
                capability: None,
                message: format!("系统截屏接口调用失败：{}", detail),
                guidance: None,
                at,
            },
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: FailureKind::Internal,
            capability: None,
            message: message.into(),
            guidance: None,
            at: chrono::Utc::now().timestamp_millis(),
        }
    }

    /// 转为内核错误；权限指引一并写入消息，IPC 调用方不需要再查询状态即可展示。
    pub fn to_core_error(&self) -> CoreError {
        match &self.guidance {
            Some(guidance) => CoreError::Internal(anyhow!("{}。{}", self.message, guidance)),
            None => CoreError::Internal(anyhow!("{}", self.message)),
        }
    }
}

/// 模块内部的业务错误统一包装为 `CoreError::Internal`。
pub(crate) fn internal(message: impl std::fmt::Display) -> CoreError {
    CoreError::Internal(anyhow!("{}", message))
}
