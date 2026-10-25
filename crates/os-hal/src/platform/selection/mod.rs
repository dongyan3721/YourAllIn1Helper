//! 原生跨平台选区提取模块与模拟复制降级接口契约。
//!
//! 职责：
//! 1. 优先调用当前操作系统的原生 Accessibility API (macOS AXUIElement / Windows UIAutomation)。
//! 2. 模拟复制降级契约：由于共享 EventBus 剪贴板屏蔽机制当前为单 AtomicBool 并发不安全，
//!    由 Codex 统一重构为具名所有者与 RAII 租约。在此之前，模拟复制通道保持显式禁用 (Unsupported)，
//!    不冒充已具备安全模拟复制能力，先完成原生选区读取闭环。

pub mod macos;
pub mod windows;

use super::NativeOsHal;
use crate::{
    traits::SelectionExtractor,
    types::{HalError, HalResult, SelectionTextResult},
    WindowController,
};

impl SelectionExtractor for NativeOsHal {
    fn is_available(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            // 只要支持 macOS 平台即具备提取能力（是否授权在调用时准确返回）
            true
        }
        #[cfg(target_os = "windows")]
        {
            true
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            false
        }
    }

    fn extract_selected_text(&self) -> HalResult<Option<SelectionTextResult>> {
        // 1. 获取当前前台窗口上下文（保持前台焦点，不激活任何自身窗口）
        let (fg_pid, fg_name) = match self.get_foreground_window() {
            Ok(fg) => (Some(fg.process_id), Some(fg.process_name)),
            Err(_) => (None, None),
        };

        // 2. 优先走原生无障碍通道
        #[cfg(target_os = "macos")]
        {
            match macos::extract_selected_text_macos(fg_pid, fg_name) {
                Ok(result) => return Ok(result),
                Err(HalError::PermissionDenied {
                    capability,
                    guidance,
                }) => {
                    return Err(HalError::PermissionDenied {
                        capability,
                        guidance,
                    });
                }
                Err(HalError::Unsupported(_)) => {
                    // 原生通道不支持此应用（如某些终端或非标准画布控件）
                    // 此时需检查模拟复制降级通道
                }
                Err(err) => return Err(err),
            }
        }

        #[cfg(target_os = "windows")]
        {
            match windows::extract_selected_text_windows(fg_pid, fg_name) {
                Ok(result) => return Ok(result),
                Err(err) => return Err(err),
            }
        }

        // 3. 模拟复制降级检查
        // 契约说明：共享剪贴板租约由 Codex 负责重构，目前仅有并发不安全的 AtomicBool。
        // 此降级在此明确禁用，避免污染用户真实剪贴板或打断密码本等并行操作。
        Err(HalError::Unsupported(
            "simulated_copy_fallback_disabled_pending_clipboard_lease",
        ))
    }
}
