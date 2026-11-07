//! Windows 前台应用、键鼠空闲、锁屏检测与 Core Audio 默认输出音频状态探测。

use crate::platform::NativeOsHal;
use crate::{ActiveAppSnapshot, HalError, HalResult};

#[cfg(target_os = "windows")]
use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
#[cfg(target_os = "windows")]
use windows::Win32::Media::Audio::{
    eConsole, eRender, AudioSessionStateActive, IAudioSessionManager2, IMMDevice,
    IMMDeviceEnumerator, MMDeviceEnumerator,
};
#[cfg(target_os = "windows")]
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
};
#[cfg(target_os = "windows")]
use windows::Win32::System::RemoteDesktop::{
    WTSFreeMemory, WTSQuerySessionInformationW, WTSSessionInfoEx, WTSINFOEXW,
    WTS_CURRENT_SESSION, WTS_SESSIONSTATE_LOCK, WTS_SESSIONSTATE_UNLOCK,
};
#[cfg(target_os = "windows")]
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, OpenInputDesktop, DESKTOP_CONTROL_FLAGS, DESKTOP_SWITCHDESKTOP,
};

/// RAII 守卫：确保在任何分支退出时都会释放 WTS 分配的内存缓冲区
#[cfg(target_os = "windows")]
struct WtsBufferGuard(windows::core::PWSTR);

#[cfg(target_os = "windows")]
impl Drop for WtsBufferGuard {
    fn drop(&mut self) {
        if !self.0 .0.is_null() {
            unsafe {
                WTSFreeMemory(self.0 .0 as _);
            }
        }
    }
}

/// 查询系统 Terminal Services 会话扩展信息中的 SessionFlags
#[cfg(target_os = "windows")]
pub(super) fn query_wts_session_flags() -> HalResult<u32> {
    unsafe {
        let mut buffer = windows::core::PWSTR::null();
        let mut bytes_returned = 0u32;
        let res = WTSQuerySessionInformationW(
            None,
            WTS_CURRENT_SESSION,
            WTSSessionInfoEx,
            &mut buffer,
            &mut bytes_returned,
        );
        let _guard = WtsBufferGuard(buffer);
        if let Err(err) = res {
            return Err(HalError::NativeCallFailed(format!(
                "WTSQuerySessionInformationW WTSSessionInfoEx failed: {err}"
            )));
        }
        if buffer.0.is_null() || bytes_returned < std::mem::size_of::<WTSINFOEXW>() as u32 {
            return Err(HalError::NativeCallFailed(
                "WTSQuerySessionInformationW returned null or truncated WTSINFOEXW".to_string(),
            ));
        }

        let info_ptr = buffer.0 as *const WTSINFOEXW;
        let info = &*info_ptr;
        if info.Level != 1 {
            return Err(HalError::NativeCallFailed(format!(
                "WTSSessionInfoEx unsupported level {}",
                info.Level
            )));
        }

        let session_flags = info.Data.WTSInfoExLevel1.SessionFlags as u32;
        Ok(session_flags)
    }
}

/// 检查当前会话的交互输入桌面访问权限
#[cfg(target_os = "windows")]
pub(super) fn check_desktop_switch_permission() -> HalResult<()> {
    unsafe {
        match OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_SWITCHDESKTOP) {
            Ok(desktop) => {
                let _ = CloseDesktop(desktop);
                Ok(())
            }
            Err(err) => {
                let code = err.code().0 as u32;
                // ERROR_ACCESS_DENIED (0x80070005 或 5) 表示当前处于不可访问的受保护锁屏/凭据桌面
                if code == 0x80070005 || code == 5 {
                    Err(HalError::PermissionDenied {
                        capability: "desktop_switch",
                        guidance: "Input desktop is restricted (locked or secure desktop)".to_string(),
                    })
                } else {
                    Err(HalError::NativeCallFailed(format!(
                        "OpenInputDesktop failed: {err}"
                    )))
                }
            }
        }
    }
}

/// 纯逻辑决策函数：根据 SessionFlags 与输入桌面权限判定锁屏状态。
/// 拆出此纯函数以便单元测试覆盖所有分支（已锁定、已解锁、未知标志、截断、错误等）。
pub(crate) fn evaluate_windows_screen_locked(
    session_flags_res: HalResult<u32>,
    desktop_switch_res: HalResult<()>,
) -> HalResult<bool> {
    let session_flags = session_flags_res?;
    match session_flags {
        WTS_SESSIONSTATE_LOCK => Ok(true),
        WTS_SESSIONSTATE_UNLOCK => match desktop_switch_res {
            Ok(()) => Ok(false),
            Err(HalError::PermissionDenied { .. }) => Ok(true),
            Err(err) => Err(err),
        },
        _ => Err(HalError::NativeCallFailed(format!(
            "WTSSessionInfoEx returned unknown SessionFlags: {session_flags}"
        ))),
    }
}

/// 探测 1：通过 IAudioSessionManager2 枚举活跃会话
#[cfg(target_os = "windows")]
unsafe fn probe_audio_session_active(device: &IMMDevice) -> HalResult<bool> {
    let manager = device
        .Activate::<IAudioSessionManager2>(CLSCTX_ALL, None)
        .map_err(|e| HalError::NativeCallFailed(format!("Activate IAudioSessionManager2: {e}")))?;

    let session_enum = manager
        .GetSessionEnumerator()
        .map_err(|e| HalError::NativeCallFailed(format!("GetSessionEnumerator: {e}")))?;

    let count = session_enum
        .GetCount()
        .map_err(|e| HalError::NativeCallFailed(format!("GetCount: {e}")))?;

    let mut last_query_error = None;
    for i in 0..count {
        match session_enum.GetSession(i) {
            Ok(control) => match control.GetState() {
                Ok(state) => {
                    if state == AudioSessionStateActive {
                        return Ok(true);
                    }
                }
                Err(e) => {
                    last_query_error = Some(HalError::NativeCallFailed(format!(
                        "GetState failed for audio session index {i}: {e}"
                    )));
                }
            },
            Err(e) => {
                last_query_error = Some(HalError::NativeCallFailed(format!(
                    "GetSession failed for audio session index {i}: {e}"
                )));
            }
        }
    }

    if let Some(err) = last_query_error {
        Err(err)
    } else {
        Ok(false)
    }
}

/// 探测 2：通过 IAudioMeterInformation 测量实时峰值电平
#[cfg(target_os = "windows")]
unsafe fn probe_audio_meter_active(device: &IMMDevice) -> HalResult<bool> {
    let meter = device
        .Activate::<IAudioMeterInformation>(CLSCTX_ALL, None)
        .map_err(|e| HalError::NativeCallFailed(format!("Activate IAudioMeterInformation: {e}")))?;

    let peak = meter
        .GetPeakValue()
        .map_err(|e| HalError::NativeCallFailed(format!("GetPeakValue: {e}")))?;

    Ok(peak > 0.0001)
}

/// 纯逻辑决策函数：合并两条音频探测路径的结果。
/// 任一成功检测到活跃则返回 Ok(true)；两路皆成功且无活跃则返回 Ok(false)；
/// 一路成功一路失败时信任成功的非活跃结果；两路皆遭遇系统调用错误时保守返回错误，绝不吞错冒充 false。
pub(crate) fn evaluate_windows_audio_active(
    session_probe: HalResult<bool>,
    meter_probe: HalResult<bool>,
) -> HalResult<bool> {
    match (session_probe, meter_probe) {
        // 任一探测证实有音频流活动，立即判定为活跃
        (Ok(true), _) | (_, Ok(true)) => Ok(true),
        // 两者皆成功且均未检测到音频活动
        (Ok(false), Ok(false)) => Ok(false),
        // 会话探测成功确认无活跃流，电平探测失败（例如特定端点不支持峰值计量）：信任会话结果
        (Ok(false), Err(_)) => Ok(false),
        // 电平探测成功确认无输出电平，会话探测失败（例如特定端点不支持会话枚举）：信任电平结果
        (Err(_), Ok(false)) => Ok(false),
        // 两条探测途径全部遭遇错误：不可判定，必须返回真实错误，严禁吞错冒充 false
        (Err(err1), Err(err2)) => Err(HalError::NativeCallFailed(format!(
            "Both Windows audio probes failed: session probe ({err1}); meter probe ({err2})"
        ))),
    }
}

impl NativeOsHal {
    /// 读取 Windows 当前前台活动窗口所属应用快照
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

    /// 查询系统自上次键盘或鼠标输入以来的空闲时间（秒）
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

    /// 可靠轮询当前工作站是否处于锁屏或无法交互的桌面状态。
    ///
    /// 查询 WTSSessionInfoEx 的 SessionFlags 验证锁定标志（严格区分活跃会话与锁屏状态），
    /// 并结合 OpenInputDesktop 进行桌面权限交叉校验；无法可靠读取时明确返回错误，绝不伪造未锁屏。
    pub(super) fn usage_screen_locked_windows(&self) -> HalResult<bool> {
        let flags_res = query_wts_session_flags();
        let desktop_res = check_desktop_switch_permission();
        evaluate_windows_screen_locked(flags_res, desktop_res)
    }

    /// 通过真实 Windows Core Audio (WASAPI) 查询默认输出设备是否有音频流正在播放。
    ///
    /// 结合 IAudioSessionManager2 活跃会话检测与 IAudioMeterInformation 实时峰值电平，
    /// 无默认输出设备时与 macOS 行为一致返回 Ok(false)；两条探测均失败时明确报错，不吞错冒充 false。
    pub(super) fn usage_audio_active_windows(&self) -> HalResult<bool> {
        // 处理 COM 初始化，使用 RAII Guard 确保仅在本函数成功初始化时配对卸载
        struct ComGuard(bool);
        impl Drop for ComGuard {
            fn drop(&mut self) {
                if self.0 {
                    unsafe {
                        CoUninitialize();
                    }
                }
            }
        }

        unsafe {
            let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
            // S_OK (0) 或 S_FALSE (1) 说明本线程初始化了 COM（或重复初始化），需要配对卸载；
            // RPC_E_CHANGED_MODE (0x80010106) 说明当前线程已被其它模块初始化（例如 STA），可继续使用但不可反初始化。
            let _com_guard = if hr.is_ok() {
                ComGuard(true)
            } else if hr.0 == 0x80010106_u32 as i32 {
                ComGuard(false)
            } else {
                return Err(HalError::NativeCallFailed(format!(
                    "CoInitializeEx failed with HRESULT 0x{:08X}",
                    hr.0
                )));
            };

            let enumerator: IMMDeviceEnumerator = match CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) {
                Ok(e) => e,
                Err(err) => {
                    return Err(HalError::NativeCallFailed(format!(
                        "CoCreateInstance MMDeviceEnumerator failed: {err}"
                    )));
                }
            };

            // 获取默认输出音频设备（渲染端点）
            let device = match enumerator.GetDefaultAudioEndpoint(eRender, eConsole) {
                Ok(d) => d,
                Err(err) => {
                    let win32_code = err.code().0 as u32;
                    // E_NOTFOUND (0x80070490) 或找不到设备：与 macOS 一致，没有默认输出设备时视为未播放
                    if win32_code == 0x80070490 || win32_code == 0x80070002 {
                        return Ok(false);
                    }
                    return Err(HalError::NativeCallFailed(format!(
                        "GetDefaultAudioEndpoint failed: {err}"
                    )));
                }
            };

            let session_probe = probe_audio_session_active(&device);
            let meter_probe = probe_audio_meter_active(&device);

            evaluate_windows_audio_active(session_probe, meter_probe)
        }
    }
}
