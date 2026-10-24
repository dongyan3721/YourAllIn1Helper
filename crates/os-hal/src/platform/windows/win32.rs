//! Win32 外部窗口激活和置顶实现，与 macOS 分开维护。
use super::*;
impl NativeOsHal {
    pub(super) fn activate_win32_window(&self, info: &ExternalWindowInfo) -> HalResult<()> {
        use windows::Win32::{Foundation::HWND, UI::WindowsAndMessaging::SetForegroundWindow};
        unsafe {
            SetForegroundWindow(HWND(info.handle as *mut std::ffi::c_void))
                .ok()
                .map_err(native_error)?;
        }
        return Ok(());
    }
    pub(super) fn set_win32_topmost(&self, handle: WindowHandleId, topmost: bool) -> HalResult<()> {
        use windows::Win32::{
            Foundation::HWND,
            UI::WindowsAndMessaging::{
                SetWindowPos, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
            },
        };
        unsafe {
            SetWindowPos(
                HWND(handle as *mut std::ffi::c_void),
                if topmost {
                    HWND_TOPMOST
                } else {
                    HWND_NOTOPMOST
                },
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
            .map_err(native_error)?;
        }
        return Ok(());
    }
}
