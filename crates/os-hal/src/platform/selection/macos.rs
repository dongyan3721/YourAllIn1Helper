//! macOS 平台原生无障碍（Accessibility / AXUIElement）选区提取适配器。
//!
//! 职责：
//! 1. 严格检查系统无障碍权限（AXIsProcessTrusted），若无权限准确返回 PermissionDenied 及中文指引。
//! 2. 保持目标应用原焦点，不主动弹出窗口抢占焦点。
//! 3. 通过 AXUIElementCopyAttributeValue 查询当前焦点控件及选中文本。
//! 4. 选区为空或不支持时准确返回 Ok(None) 或 Unsupported，严禁虚构选区文本。

use crate::types::{HalError, HalResult, SelectionSource, SelectionTextResult};
use std::ffi::c_void;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXUIElementCreateApplication(pid: i32) -> *mut c_void;
    fn AXUIElementCopyAttributeValue(
        element: *mut c_void,
        attribute: *const c_void,
        value: *mut *mut c_void,
    ) -> i32;
    fn CFRelease(cf: *const c_void);
    fn CFGetTypeID(cf: *const c_void) -> usize;
    fn CFStringGetTypeID() -> usize;
}

// macOS AX 常量定义
const K_AX_ERROR_SUCCESS: i32 = 0;
const K_AX_ERROR_NO_VALUE: i32 = -25212;
const K_AX_ERROR_ATTRIBUTE_UNSUPPORTED: i32 = -25205;
const K_AX_ERROR_NOT_IMPLEMENTED: i32 = -25208;
const K_AX_ERROR_CANNOT_COMPLETE: i32 = -25204;
const K_AX_ERROR_API_DISABLED: i32 = -25211;

/// 检查 macOS 无障碍权限是否已授予。
pub fn is_accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// 从当前前台应用的原生焦点控件读取选中文本。
pub fn extract_selected_text_macos(
    foreground_pid: Option<u32>,
    foreground_name: Option<String>,
) -> HalResult<Option<SelectionTextResult>> {
    // 1. 权限前置核验
    if !is_accessibility_trusted() {
        return Err(HalError::PermissionDenied {
            capability: "accessibility",
            guidance: "未授予辅助功能控制权限，请在「系统设置 → 隐私与安全性 → 辅助功能」中为应用开启权限，以便安全提取选区文本。".to_string(),
        });
    }

    // 2. 获取目标前台进程 PID
    let pid = match foreground_pid {
        Some(p) if p > 0 => p as i32,
        _ => {
            // 通过 NSWorkspace 获取前台应用
            use objc2_app_kit::NSWorkspace;
            let workspace = NSWorkspace::sharedWorkspace();
            let front_app = workspace.frontmostApplication();
            match front_app {
                Some(app) => app.processIdentifier(),
                None => return Ok(None),
            }
        }
    };

    if pid <= 0 || pid == std::process::id() as i32 {
        // 若焦点为自身或无效，不读取
        return Ok(None);
    }

    unsafe {
        // 3. 创建目标应用的 AXUIElement
        let app_elem = AXUIElementCreateApplication(pid);
        if app_elem.is_null() {
            return Ok(None);
        }

        // 构造属性常量 CFString
        use objc2_core_foundation::CFString;
        let cf_focused_attr = CFString::from_str("AXFocusedUIElement");
        let cf_selected_text_attr = CFString::from_str("AXSelectedText");

        let mut focused_elem: *mut c_void = std::ptr::null_mut();
        let focused_status = AXUIElementCopyAttributeValue(
            app_elem,
            cf_focused_attr.as_ref() as *const CFString as *const c_void,
            &mut focused_elem,
        );

        if focused_status != K_AX_ERROR_SUCCESS || focused_elem.is_null() {
            CFRelease(app_elem);
            if focused_status == K_AX_ERROR_NO_VALUE {
                return Ok(None);
            }
            if focused_status == K_AX_ERROR_API_DISABLED {
                return Err(HalError::PermissionDenied {
                    capability: "accessibility",
                    guidance: "目标应用无障碍 API 已禁用，无法读取选区".to_string(),
                });
            }
            return Err(HalError::Unsupported("accessibility_focused_element"));
        }

        // 4. 从焦点元素提取 AXSelectedText
        let mut selected_val: *mut c_void = std::ptr::null_mut();
        let text_status = AXUIElementCopyAttributeValue(
            focused_elem,
            cf_selected_text_attr.as_ref() as *const CFString as *const c_void,
            &mut selected_val,
        );

        let result = if text_status == K_AX_ERROR_SUCCESS && !selected_val.is_null() {
            if CFGetTypeID(selected_val) != CFStringGetTypeID() {
                CFRelease(selected_val);
                CFRelease(focused_elem);
                CFRelease(app_elem);
                return Err(HalError::Unsupported("accessibility_selected_text_type"));
            }
            let cf_str_ref = &*(selected_val as *const CFString);
            let text = cf_str_ref.to_string();
            CFRelease(selected_val);

            let trimmed = text.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(SelectionTextResult {
                    text,
                    source: SelectionSource::AccessibilityApi,
                    source_app_name: foreground_name,
                }))
            }
        } else if text_status == K_AX_ERROR_NO_VALUE {
            // 当前元素没有选中文本，属于正常状态
            Ok(None)
        } else if text_status == K_AX_ERROR_API_DISABLED {
            Err(HalError::PermissionDenied {
                capability: "accessibility",
                guidance: "目标应用无障碍 API 已禁用，无法读取选区".to_string(),
            })
        } else if text_status == K_AX_ERROR_ATTRIBUTE_UNSUPPORTED
            || text_status == K_AX_ERROR_NOT_IMPLEMENTED
            || text_status == K_AX_ERROR_CANNOT_COMPLETE
        {
            // 元素不支持选区读取
            Err(HalError::Unsupported("accessibility_selected_text"))
        } else {
            Err(HalError::NativeCallFailed(format!(
                "AXSelectedText 返回错误: {}",
                text_status
            )))
        };

        CFRelease(focused_elem);
        CFRelease(app_elem);
        result
    }
}
