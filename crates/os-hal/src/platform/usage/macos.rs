//! macOS 真实前台应用、空闲与锁屏/息屏检测。
use crate::platform::NativeOsHal;
use crate::{ActiveAppSnapshot, HalError, HalResult};

impl NativeOsHal {
    pub(super) fn usage_active_app_macos(&self) -> HalResult<ActiveAppSnapshot> {
        use objc2_app_kit::NSWorkspace;

        let workspace = NSWorkspace::sharedWorkspace();
        let app = workspace
            .frontmostApplication()
            .ok_or_else(|| HalError::NotFound("No frontmost macOS application".to_string()))?;
        let process_id = app.processIdentifier();
        let app_name = app
            .localizedName()
            .map(|name| name.to_string())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| format!("PID {process_id}"));
        let executable_path = app
            .executableURL()
            .and_then(|url| url.path())
            .map(|path| path.to_string())
            .filter(|path| !path.trim().is_empty());

        // 稳定 Bundle ID 优先；无 Bundle ID 时退回可执行文件绝对路径，绝不拼接进程名冒充 ID。
        let app_id = app
            .bundleIdentifier()
            .map(|identifier| identifier.to_string())
            .filter(|identifier| !identifier.trim().is_empty())
            .or_else(|| executable_path.clone())
            .unwrap_or_else(|| format!("pid:{process_id}"));

        let window_title = front_window_title(process_id as u32).unwrap_or_default();
        Ok(ActiveAppSnapshot {
            app_id,
            app_name,
            window_title,
            executable_path,
            process_id: process_id.max(0) as u32,
        })
    }

    pub(super) fn usage_idle_seconds_macos(&self) -> HalResult<u64> {
        use objc2_core_graphics::{CGEventSource, CGEventSourceStateID, CGEventType};

        // kCGAnyInputEventType = ~0：任意键盘/鼠标/触控板事件。
        let seconds = CGEventSource::seconds_since_last_event_type(
            CGEventSourceStateID::CombinedSessionState,
            CGEventType(u32::MAX),
        );
        if seconds.is_finite() && seconds >= 0.0 {
            Ok(seconds.floor() as u64)
        } else {
            Err(HalError::NativeCallFailed(
                "CGEventSourceSecondsSinceLastEventType returned an invalid interval".to_string(),
            ))
        }
    }

    pub(super) fn usage_screen_locked_macos(&self) -> HalResult<bool> {
        use objc2_core_foundation::CFBoolean;
        use objc2_core_graphics::{
            CGDisplayIsAsleep, CGMainDisplayID, CGSessionCopyCurrentDictionary,
        };
        use std::ffi::c_void;

        // 显示睡眠（合盖/息屏）与用户会话锁定都视为不可计时。
        let display_asleep = CGDisplayIsAsleep(CGMainDisplayID());
        let session_locked = match CGSessionCopyCurrentDictionary() {
            Some(dictionary) => unsafe {
                let key = objc2_core_foundation::CFString::from_str("CGSSessionScreenIsLocked");
                let value = dictionary
                    .value(key.as_ref() as *const objc2_core_foundation::CFString as *const c_void);
                if value.is_null() {
                    false
                } else {
                    (*(value as *const CFBoolean)).value()
                }
            },
            None => {
                return Err(HalError::NativeCallFailed(
                    "CGSessionCopyCurrentDictionary returned no session".to_string(),
                ));
            }
        };
        Ok(display_asleep || session_locked)
    }
}

/// 读取前台应用最上层普通窗口的标题；无屏幕录制权限时系统可能返回空名称。
fn front_window_title(pid: u32) -> Option<String> {
    use objc2_core_foundation::{CFDictionary, CFNumber, CFNumberType, CFString};
    use objc2_core_graphics::{CGWindowListCopyWindowInfo, CGWindowListOption};
    use std::ffi::c_void;

    unsafe {
        let windows = CGWindowListCopyWindowInfo(
            CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements,
            0,
        )?;
        for index in 0..windows.count() {
            let dictionary = windows.value_at_index(index) as *const CFDictionary;
            if dictionary.is_null() {
                continue;
            }
            let dictionary = &*dictionary;
            let read_number = |key: &str| -> Option<i32> {
                let cf_key = CFString::from_str(key);
                let value = dictionary.value(cf_key.as_ref() as *const CFString as *const c_void);
                if value.is_null() {
                    return None;
                }
                let mut number: i32 = 0;
                (*(value as *const CFNumber))
                    .value(CFNumberType::IntType, &mut number as *mut _ as *mut c_void)
                    .then_some(number)
            };
            if read_number("kCGWindowOwnerPID") != Some(pid as i32) {
                continue;
            }
            if read_number("kCGWindowLayer").unwrap_or(0) != 0 {
                continue;
            }
            let cf_key = CFString::from_str("kCGWindowName");
            let value = dictionary.value(cf_key.as_ref() as *const CFString as *const c_void);
            if value.is_null() {
                continue;
            }
            let title = (*(value as *const CFString)).to_string();
            if !title.trim().is_empty() {
                return Some(title);
            }
        }
    }
    None
}
