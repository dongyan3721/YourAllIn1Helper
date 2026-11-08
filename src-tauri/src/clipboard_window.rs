//! Clipboard popup window policy.
//!
//! - macOS: keep Dashboard out of the key-window fallback during external quick paste.
//! - Windows: make sure the popup (and its WebView2 content) really owns keyboard focus.
//!   When the hotkey is pressed inside an Electron/VS Code style IDE, a freshly created
//!   transient window may be denied foreground activation. Then the arrow keys keep
//!   moving the IDE caret instead of the candidate list. We ask Windows once, verify the
//!   result for a bounded time, and fail loudly instead of faking input or forcing focus.
use std::time::Duration;

#[cfg(target_os = "macos")]
use tauri::Manager;
use tauri::{AppHandle, WebviewWindow};

pub(crate) fn prepare_popup(app: &AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let external = objc2_app_kit::NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .is_some_and(|front| front.processIdentifier() as u32 != std::process::id());
        if external {
            if let Some(dashboard) = app.get_webview_window("dashboard") {
                // Do not show it again on popup destruction: that would steal focus
                // or flash before the destination application is reactivated.
                dashboard.hide().map_err(|error| error.to_string())?;
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
    Ok(())
}

/// Give an already existing popup keyboard focus again.
pub(crate) fn focus_popup(window: &WebviewWindow) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        windows_focus::acquire(window)
    }
    #[cfg(not(target_os = "windows"))]
    {
        window.set_focus().map_err(|error| error.to_string())
    }
}

/// Verify a just-created popup owns the foreground. No-op outside Windows, where
/// `focused(true)` at creation already activates the window.
pub(crate) fn present_new_popup(window: &WebviewWindow) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let result = windows_focus::acquire(window);
        // 仅调试构建输出窗口激活结果，不记录剪贴板正文。
        #[cfg(debug_assertions)]
        eprintln!("Clipboard popup activation: {result:?}");
        result
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = window;
        Ok(())
    }
}

/// Windows 顶层窗口交焦给 WebView2 子控件时也可能收到 Focused(false)。
/// 只有真正离开本候选窗的前台才应销毁，不能把内部焦点切换当作用户取消。
pub(crate) fn owns_foreground(window: &tauri::Window) -> bool {
    #[cfg(target_os = "windows")]
    {
        windows_focus::owns_foreground(window)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = window;
        false
    }
}

/// Upper bound for waiting until Windows reports the popup as the foreground window.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const FOREGROUND_TIMEOUT: Duration = Duration::from_millis(250);
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const FOREGROUND_POLL: Duration = Duration::from_millis(10);

/// Polls `is_foreground` until it holds or `wait_again` reports the deadline is over.
/// The state is always re-checked after every wait, and never after a failed wait.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn wait_until_foreground(
    mut is_foreground: impl FnMut() -> bool,
    mut wait_again: impl FnMut() -> bool,
) -> bool {
    loop {
        if is_foreground() {
            return true;
        }
        if !wait_again() {
            return false;
        }
    }
}

/// A null handle can never be the popup, even if Windows reports no foreground window.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn popup_is_foreground(popup: isize, foreground: isize) -> bool {
    popup != 0 && popup == foreground
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn foreground_denied_message(popup: isize, foreground: isize, foreground_pid: u32) -> String {
    if foreground == 0 {
        format!(
            "Windows 没有把剪贴板候选窗 (HWND {popup:#x}) 设为前台窗口，当前没有前台窗口；已取消呼出"
        )
    } else {
        format!(
            "Windows 拒绝把剪贴板候选窗 (HWND {popup:#x}) 设为前台窗口，前台仍是进程 {foreground_pid} 的窗口 \
             (HWND {foreground:#x})；已取消呼出，避免方向键落到其他应用"
        )
    }
}

#[cfg(target_os = "windows")]
mod windows_focus {
    use super::*;
    use ::windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
        },
    };
    use std::{ffi::c_void, time::Instant};

    fn foreground_handle() -> isize {
        unsafe { GetForegroundWindow() }.0 as isize
    }

    fn process_of(hwnd: isize) -> u32 {
        let mut process_id = 0;
        unsafe { GetWindowThreadProcessId(HWND(hwnd as *mut c_void), Some(&mut process_id)) };
        process_id
    }

    pub(super) fn owns_foreground(window: &tauri::Window) -> bool {
        window
            .hwnd()
            .is_ok_and(|hwnd| popup_is_foreground(hwnd.0 as isize, foreground_handle()))
    }

    pub(super) fn acquire(window: &WebviewWindow) -> Result<(), String> {
        // Tauri exposes the HWND of its own `windows` crate version; compare as integers.
        let popup = window.hwnd().map_err(|error| error.to_string())?.0 as isize;
        window.show().map_err(|error| error.to_string())?;

        if !popup_is_foreground(popup, foreground_handle()) {
            // Ask exactly once through the documented API. Deliberately no Alt-key
            // injection (Tauri's `set_focus` does that on refusal and would deliver the
            // key to whatever window is in front) and no thread-input attachment.
            let _ = unsafe { SetForegroundWindow(HWND(popup as *mut c_void)) };
        }

        let deadline = Instant::now() + FOREGROUND_TIMEOUT;
        let acquired = wait_until_foreground(
            || popup_is_foreground(popup, foreground_handle()),
            || {
                if Instant::now() >= deadline {
                    return false;
                }
                std::thread::sleep(FOREGROUND_POLL);
                true
            },
        );
        if !acquired {
            let foreground = foreground_handle();
            let reason = foreground_denied_message(popup, foreground, process_of(foreground));
            // 调试构建直接输出失败原因，便于桌面专项区分激活被拒和 WebView 焦点问题。
            #[cfg(debug_assertions)]
            eprintln!("{reason}");
            return Err(reason);
        }

        // The top-level window being active does not move keyboard focus into WebView2.
        let webview: &tauri::Webview = window.as_ref();
        webview.set_focus().map_err(|error| error.to_string())
    }
}

#[cfg(test)]
#[path = "../tests/unit/clipboard_window.rs"]
mod tests;
