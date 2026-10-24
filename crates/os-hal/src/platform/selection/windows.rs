//! Windows 平台原生无障碍（UIAutomation / TextPattern）选区提取适配器。
//!
//! 在 Windows 下应通过 IUIAutomation::GetFocusedElement + TextPattern::GetSelection 获取选区。
//! 当前架构下若尚未加载 COM 依赖或未运行在 Windows 上，明确报告 Unsupported。

use crate::types::{HalError, HalResult, SelectionTextResult};

/// Windows UIAutomation 选区提取实现。
#[allow(dead_code)]
pub fn extract_selected_text_windows(
    _foreground_pid: Option<u32>,
    _foreground_name: Option<String>,
) -> HalResult<Option<SelectionTextResult>> {
    #[cfg(target_os = "windows")]
    {
        // Windows 平台具体 UIAutomation 实现（后续补充完整 COM 调用链）
        Err(HalError::Unsupported("windows_uiautomation_selection"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err(HalError::Unsupported(
            "windows_selection_extraction_on_non_windows",
        ))
    }
}
