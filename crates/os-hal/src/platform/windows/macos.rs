//! macOS 窗口枚举及应用激活实现。
use super::*;
use std::process::Command;

impl NativeOsHal {
    pub(super) fn native_windows_macos(&self) -> HalResult<Vec<ExternalWindowInfo>> {
        use objc2_app_kit::NSRunningApplication;
        use objc2_core_foundation::{CFDictionary, CFNumber, CFNumberType, CFString, CGRect};
        use objc2_core_graphics::{
            CGRectMakeWithDictionaryRepresentation, CGWindowListCopyWindowInfo, CGWindowListOption,
        };
        use std::ffi::c_void;

        let own_pid = std::process::id();
        let mut result = Vec::new();

        unsafe {
            // 获取所有屏幕上显示的窗口列表（排除桌面元素）
            let cf_array = match CGWindowListCopyWindowInfo(
                CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements,
                0,
            ) {
                Some(arr) => arr,
                None => return Ok(result),
            };

            let count = cf_array.count();
            for i in 0..count {
                let dict_ref = cf_array.value_at_index(i) as *const CFDictionary;
                if dict_ref.is_null() {
                    continue;
                }
                let dict = &*dict_ref;

                let get_num = |key: &str| -> Option<i32> {
                    let cf_key = CFString::from_str(key);
                    let val = dict.value(cf_key.as_ref() as *const CFString as *const c_void);
                    if val.is_null() {
                        return None;
                    }
                    let mut n: i32 = 0;
                    if (*(val as *const CFNumber))
                        .value(CFNumberType::IntType, &mut n as *mut _ as *mut c_void)
                    {
                        Some(n)
                    } else {
                        None
                    }
                };

                let get_str = |key: &str| -> Option<String> {
                    let cf_key = CFString::from_str(key);
                    let val = dict.value(cf_key.as_ref() as *const CFString as *const c_void);
                    if val.is_null() {
                        return None;
                    }
                    Some((*(val as *const CFString)).to_string())
                };

                // 提取窗口 handle (kCGWindowNumber)
                let handle = match get_num("kCGWindowNumber") {
                    Some(id) if id > 0 => id as u64,
                    _ => continue,
                };

                // 提取进程 PID
                let pid = get_num("kCGWindowOwnerPID").unwrap_or(0) as u32;
                if pid == 0 || pid == own_pid {
                    continue;
                }

                // 核心过滤：macOS 标准用户级应用窗口层级为 0
                // 剔除 WindowServer (Layer 24/25), Dock (Layer 20), 通知中心 (负数 Layer)
                let layer = get_num("kCGWindowLayer").unwrap_or(0);
                if layer != 0 {
                    continue;
                }

                // 过滤系统全局组件所有者
                let owner_name = get_str("kCGWindowOwnerName").unwrap_or_default();
                if owner_name == "Window Server"
                    || owner_name == "Dock"
                    || owner_name == "ControlCenter"
                    || owner_name == "NotificationCenter"
                    || owner_name == "loginwindow"
                    || owner_name == "Spotlight"
                {
                    continue;
                }

                // 解析窗口真实矩形尺寸
                let mut rect = CGRect::ZERO;
                let bounds_key = CFString::from_str("kCGWindowBounds");
                let bounds_val =
                    dict.value(bounds_key.as_ref() as *const CFString as *const c_void);
                if !bounds_val.is_null() {
                    CGRectMakeWithDictionaryRepresentation(
                        Some(&*(bounds_val as *const CFDictionary)),
                        &mut rect,
                    );
                }
                let width = rect.size.width as u32;
                let height = rect.size.height as u32;
                if width < 32 || height < 32 {
                    continue;
                }

                // 通过 NSRunningApplication 获取系统准确本地化应用名称
                let running_app =
                    NSRunningApplication::runningApplicationWithProcessIdentifier(pid as i32);
                let app_name = running_app
                    .as_ref()
                    .and_then(|a| a.localizedName())
                    .map(|s| s.to_string())
                    .unwrap_or(owner_name);

                let raw_title = get_str("kCGWindowName").unwrap_or_default();
                let title = if !raw_title.trim().is_empty() {
                    raw_title
                } else {
                    format!("{} 窗口", app_name)
                };

                result.push(ExternalWindowInfo {
                    handle,
                    title,
                    process_name: app_name,
                    process_id: pid,
                    x: rect.origin.x as i32,
                    y: rect.origin.y as i32,
                    width,
                    height,
                    is_topmost: false,
                });
            }
        }

        Ok(result)
    }

    pub(super) fn activate_macos_window(&self, info: &ExternalWindowInfo) -> HalResult<()> {
        use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
        // 优先直接调用 NSRunningApplication 激活该应用至最前台
        if let Some(app) =
            NSRunningApplication::runningApplicationWithProcessIdentifier(info.process_id as i32)
        {
            #[allow(deprecated)]
            let _ =
                app.activateWithOptions(NSApplicationActivationOptions::ActivateIgnoringOtherApps);
            return Ok(());
        }
        // 回退到 LaunchServices open 命令
        let status = Command::new("/usr/bin/open")
            .args(["-a", &info.process_name])
            .status()
            .map_err(native_error)?;
        if !status.success() {
            return Err(HalError::NativeCallFailed(format!(
                "无法激活外部应用程序 '{}'",
                info.process_name
            )));
        }
        return Ok(());
    }
}
