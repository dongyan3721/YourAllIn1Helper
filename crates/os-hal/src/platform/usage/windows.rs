//! Windows 前台应用、键鼠空闲与锁屏检测。
//!
//! 仅在 Windows 目标编译；本轮交付环境为 macOS，Windows 分支未做实机验证。
use crate::platform::NativeOsHal;
use crate::{ActiveAppSnapshot, HalError, HalResult};

impl NativeOsHal {
    pub(super) fn usage_active_app_windows(&self) -> HalResult<ActiveAppSnapshot> {
        use windows::core::PWSTR;
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION,
        };
        use windows::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId,
        };

        unsafe {
            let window = GetForegroundWindow();
            if window.0.is_null() {
                return Err(HalError::NotFound(
                    "No foreground Windows window".to_string(),
                ));
            }
            let mut process_id = 0u32;
            GetWindowThreadProcessId(window, Some(&mut process_id));
            if process_id == 0 {
                return Err(HalError::NativeCallFailed(
                    "GetWindowThreadProcessId returned 0".to_string(),
                ));
            }

            let mut title_buffer = [0u16; 1024];
            let title_length = GetWindowTextW(window, &mut title_buffer);
            let window_title = if title_length > 0 {
                String::from_utf16_lossy(&title_buffer[..title_length as usize])
            } else {
                String::new()
            };

            let mut path_buffer = [0u16; 32_768];
            let mut path_length = path_buffer.len() as u32;
            let mut executable_path = None;
            if let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) {
                if QueryFullProcessImageNameW(
                    handle,
                    PROCESS_NAME_WIN32,
                    PWSTR(path_buffer.as_mut_ptr()),
                    &mut path_length,
                )
                .is_ok()
                    && path_length > 0
                {
                    executable_path = Some(String::from_utf16_lossy(
                        &path_buffer[..path_length as usize],
                    ));
                }
                let _ = CloseHandle(handle);
            }

            let app_name = executable_path
                .as_deref()
                .and_then(|path| path.rsplit(['\\', '/']).next())
                .filter(|name| !name.is_empty())
                .unwrap_or("Unknown Application")
                .to_string();
            // 稳定标识使用规范化后的可执行文件绝对路径，绝不把进程名当唯一 ID。
            let app_id = executable_path
                .clone()
                .unwrap_or_else(|| format!("pid:{process_id}"));

            Ok(ActiveAppSnapshot {
                app_id,
                app_name,
                window_title,
                executable_path,
                process_id,
            })
        }
    }

    pub(super) fn usage_idle_seconds_windows(&self) -> HalResult<u64> {
        use windows::Win32::System::SystemInformation::GetTickCount;
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

        unsafe {
            let mut info = LASTINPUTINFO {
                cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
                dwTime: 0,
            };
            if !GetLastInputInfo(&mut info).as_bool() {
                return Err(HalError::NativeCallFailed(
                    "GetLastInputInfo failed".to_string(),
                ));
            }
            let elapsed = GetTickCount().wrapping_sub(info.dwTime);
            Ok(u64::from(elapsed / 1_000))
        }
    }

    pub(super) fn usage_screen_locked_windows(&self) -> HalResult<bool> {
        use windows::Win32::System::StationsAndDesktops::{
            CloseDesktop, OpenInputDesktop, DESKTOP_SWITCHDESKTOP,
        };

        unsafe {
            // 锁屏时通常无法打开输入桌面；能打开则说明会话仍可交互。
            match OpenInputDesktop(0, false, DESKTOP_SWITCHDESKTOP) {
                Ok(desktop) => {
                    let _ = CloseDesktop(desktop);
                    Ok(false)
                }
                Err(_) => Ok(true),
            }
        }
    }
}
