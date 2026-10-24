//! 外部窗口能力入口，按系统选择具体实现。
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod win32;
use super::{native_error, NativeOsHal};
use crate::{ExternalWindowInfo, HalError, HalResult, WindowController, WindowHandleId};
use xcap::Window;

impl NativeOsHal {
    pub(in crate::platform) fn native_windows(&self) -> HalResult<Vec<ExternalWindowInfo>> {
        #[cfg(target_os = "macos")]
        {
            return self.native_windows_macos();
        }
        #[cfg(not(target_os = "macos"))]
        {
            let own_pid = std::process::id();
            let windows = Window::all().map_err(native_error)?;
            let mut result = Vec::new();
            for window in windows {
                let pid = window.pid().unwrap_or(0);
                if pid == own_pid || window.is_minimized().unwrap_or(false) {
                    continue;
                }
                let width = window.width().unwrap_or(0);
                let height = window.height().unwrap_or(0);
                if width < 24 || height < 24 {
                    continue;
                }
                let process_name = window
                    .app_name()
                    .unwrap_or_else(|_| "Unknown App".to_string());
                let title = window.title().unwrap_or_default();
                result.push(ExternalWindowInfo {
                    handle: window.id().unwrap_or(0) as u64,
                    title: if title.is_empty() {
                        process_name.clone()
                    } else {
                        title
                    },
                    process_name,
                    process_id: pid,
                    x: window.x().unwrap_or(0),
                    y: window.y().unwrap_or(0),
                    width,
                    height,
                    is_topmost: false,
                });
            }
            Ok(result)
        }
    }

    fn find_native_window(&self, handle: WindowHandleId) -> HalResult<ExternalWindowInfo> {
        self.native_windows()?
            .into_iter()
            .find(|window| window.handle == handle)
            .ok_or_else(|| HalError::NotFound(format!("External window {}", handle)))
    }

    fn activate_native_window(&self, handle: WindowHandleId) -> HalResult<()> {
        let info = self.find_native_window(handle)?;
        #[cfg(target_os = "macos")]
        {
            return self.activate_macos_window(&info);
        }
        #[cfg(target_os = "windows")]
        {
            return self.activate_win32_window(&info);
        }
        #[allow(unreachable_code)]
        Err(HalError::Unsupported("external_window_activation"))
    }
}

impl WindowController for NativeOsHal {
    fn supports_external_topmost(&self) -> bool {
        cfg!(target_os = "windows")
    }
    fn activate_external_window(&self, handle: WindowHandleId) -> HalResult<()> {
        self.activate_native_window(handle)
    }
    fn capture_window_data_url(&self, handle: WindowHandleId) -> HalResult<String> {
        self.capture_native_window_data_url(handle)
    }

    fn get_foreground_window(&self) -> HalResult<ExternalWindowInfo> {
        #[cfg(target_os = "macos")]
        {
            use objc2_app_kit::NSWorkspace;
            let own_pid = std::process::id();
            let workspace = NSWorkspace::sharedWorkspace();
            if let Some(front_app) = workspace.frontmostApplication() {
                let front_pid = front_app.processIdentifier() as u32;
                if front_pid != own_pid && front_pid > 0 {
                    let candidates = self.native_windows()?;
                    if let Some(win) = candidates.into_iter().find(|w| w.process_id == front_pid) {
                        return Ok(win);
                    }
                }
            }
        }
        let windows = Window::all().map_err(native_error)?;
        let own_pid = std::process::id();
        if let Some(window) = windows.iter().find(|window| {
            window.is_focused().unwrap_or(false) && window.pid().unwrap_or(0) != own_pid
        }) {
            let handle = window.id().map_err(native_error)? as u64;
            return self.find_native_window(handle);
        }
        self.native_windows()?
            .into_iter()
            .next()
            .ok_or_else(|| HalError::NotFound("No foreground external window".to_string()))
    }

    fn get_window_at_point(&self, x: i32, y: i32) -> HalResult<ExternalWindowInfo> {
        self.native_windows()?
            .into_iter()
            .find(|window| {
                x >= window.x
                    && x < window.x + window.width as i32
                    && y >= window.y
                    && y < window.y + window.height as i32
            })
            .ok_or_else(|| HalError::NotFound(format!("No window at ({}, {})", x, y)))
    }

    fn set_external_window_topmost(&self, handle: WindowHandleId, topmost: bool) -> HalResult<()> {
        #[cfg(target_os = "windows")]
        {
            return self.set_win32_topmost(handle, topmost);
        }
        #[cfg(target_os = "macos")]
        {
            // macOS does not provide a supported API to change another app's window level.
            // CGSSetWindowLevel can report success without changing WindowServer's actual layer.
            let _ = (handle, topmost);
            return Err(HalError::Unsupported("macos_external_window_topmost"));
        }
        #[allow(unreachable_code)]
        Err(HalError::Unsupported("external_window_topmost"))
    }

    fn get_window_info(&self, handle: WindowHandleId) -> HalResult<ExternalWindowInfo> {
        self.find_native_window(handle)
    }

    fn list_external_windows(&self) -> HalResult<Vec<ExternalWindowInfo>> {
        self.native_windows()
    }
}
