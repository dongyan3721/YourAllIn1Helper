//! Windows 粘贴目标记忆、前台确认与物理快捷键发送。

use super::super::{native_error, NativeOsHal};
use crate::{HalError, HalResult};
use std::{
    ffi::c_void,
    mem::size_of,
    thread,
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::HWND,
    UI::{
        Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_C,
            VK_CONTROL, VK_V,
        },
        WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId, IsWindow, SetForegroundWindow,
        },
    },
};

const RESTORE_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::platform) struct PasteTarget {
    hwnd: isize,
    process_id: u32,
}

fn target_for_remembering(
    target: PasteTarget,
    own_process_id: u32,
) -> HalResult<Option<PasteTarget>> {
    if target.process_id == own_process_id {
        return Ok(None);
    }
    if !is_external_target_for(target, own_process_id) {
        return Err(native_error("无法确认外部粘贴目标身份"));
    }
    Ok(Some(target))
}

fn is_external_target_for(target: PasteTarget, own_process_id: u32) -> bool {
    target.hwnd != 0 && is_external_process(target.process_id, own_process_id)
}

fn replace_target(
    slot: &mut Option<(isize, u32)>,
    select: impl FnOnce() -> HalResult<Option<PasteTarget>>,
) -> HalResult<()> {
    *slot = None;
    *slot = select()?.map(|target| (target.hwnd, target.process_id));
    Ok(())
}

fn window(hwnd: isize) -> HWND {
    HWND(hwnd as *mut c_void)
}

fn process_id(hwnd: HWND) -> u32 {
    let mut process_id = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
    process_id
}

fn foreground_target() -> HalResult<PasteTarget> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return Err(HalError::NotFound(
            "没有可用的前台窗口，已取消快捷键".into(),
        ));
    }
    let process_id = process_id(hwnd);
    if process_id == 0 {
        return Err(native_error("无法确认前台窗口所属进程，已取消快捷键"));
    }
    Ok(PasteTarget {
        hwnd: hwnd.0 as isize,
        process_id,
    })
}

fn is_external_target(target: PasteTarget) -> bool {
    is_external_process(target.process_id, std::process::id()) && target.hwnd != 0
}

fn is_external_process(process_id: u32, own_process_id: u32) -> bool {
    process_id != 0 && process_id != own_process_id
}

fn process_identity_matches(
    target_process_id: u32,
    actual_process_id: u32,
    own_process_id: u32,
) -> bool {
    is_external_process(target_process_id, own_process_id) && target_process_id == actual_process_id
}

fn take_remembered_target(target: Option<(isize, u32)>) -> HalResult<PasteTarget> {
    target
        .map(|(hwnd, process_id)| PasteTarget { hwnd, process_id })
        .ok_or_else(|| HalError::NotFound("没有已记忆的原粘贴目标，已取消自动粘贴".into()))
}

fn target_is_foreground(
    target: PasteTarget,
    foreground_hwnd: isize,
    foreground_process_id: u32,
) -> bool {
    target.hwnd == foreground_hwnd && target.process_id == foreground_process_id
}

/// 记录弹出剪贴板瞬态面板前的外部目标窗口。
pub(in crate::platform) fn remember_paste_target(hal: &NativeOsHal) -> HalResult<()> {
    let mut slot = hal
        .paste_target
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    replace_target(&mut slot, || {
        let target = foreground_target()?;
        let Some(target) = target_for_remembering(target, std::process::id())? else {
            return Ok(None);
        };
        if !unsafe { IsWindow(Some(window(target.hwnd))) }.as_bool() {
            return Err(HalError::NotFound("原粘贴目标窗口已失效".into()));
        }
        Ok(Some(target))
    })
}

/// 恢复并确认原 HWND 仍是前台后，才发送物理 Ctrl+V。
pub(in crate::platform) fn restore_and_paste(hal: &NativeOsHal) -> HalResult<()> {
    let remembered = hal
        .paste_target
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take();
    let target = take_remembered_target(remembered)?;

    validate_live_target(target)?;
    let hwnd = window(target.hwnd);
    let current = unsafe { GetForegroundWindow() };
    if current != hwnd {
        // SetForegroundWindow 遵守 Windows 前台激活限制；不尝试 AttachThreadInput 等绕过方式。
        let _ = unsafe { SetForegroundWindow(hwnd) };
    }

    let deadline = Instant::now() + RESTORE_TIMEOUT;
    loop {
        validate_live_target(target)?;
        let current = unsafe { GetForegroundWindow() };
        if target_is_foreground(target, current.0 as isize, process_id(current)) {
            break;
        }
        if Instant::now() >= deadline {
            return Err(native_error(
                "Windows 未将原粘贴目标恢复为前台窗口，已取消自动粘贴",
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }

    thread::sleep(Duration::from_millis(140));
    send_shortcut('v', Some(target))
}

fn validate_live_target(target: PasteTarget) -> HalResult<()> {
    if !is_external_target(target) {
        return Err(native_error("原粘贴目标属于本应用，已取消自动粘贴"));
    }
    let hwnd = window(target.hwnd);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return Err(HalError::NotFound(
            "原粘贴目标窗口已退出，已取消自动粘贴".into(),
        ));
    }
    if !process_identity_matches(target.process_id, process_id(hwnd), std::process::id()) {
        return Err(HalError::NotFound(
            "原粘贴目标 HWND 已被重用，进程身份不匹配，已取消自动粘贴".into(),
        ));
    }
    Ok(())
}

/// 将快捷键发给明确确认的外部 HWND；copy 只使用调用当下的前台目标，不切换窗口。
pub(in crate::platform) fn send_shortcut(
    key: char,
    expected_target: Option<PasteTarget>,
) -> HalResult<()> {
    let target = match expected_target {
        Some(target) => target,
        None => foreground_target()?,
    };
    validate_live_target(target)?;
    if !is_external_target(target) {
        return Err(native_error("拒绝向本应用窗口发送外部快捷键"));
    }
    let current = unsafe { GetForegroundWindow() };
    if !target_is_foreground(target, current.0 as isize, process_id(current)) {
        return Err(native_error(
            "前台窗口已变化，已取消快捷键以防发送给错误目标",
        ));
    }

    let key = match key {
        'c' => VK_C,
        'v' => VK_V,
        _ => return Err(HalError::Unsupported("windows_physical_shortcut")),
    };
    let inputs = [
        keyboard_input(VK_CONTROL, false),
        keyboard_input(key, false),
        keyboard_input(key, true),
        keyboard_input(VK_CONTROL, true),
    ];
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    if sent != inputs.len() as u32 {
        let release = [keyboard_input(key, true), keyboard_input(VK_CONTROL, true)];
        unsafe { SendInput(&release, size_of::<INPUT>() as i32) };
        return Err(native_error(format!(
            "SendInput 仅接受 {sent}/{} 个物理按键事件；快捷键未确认",
            inputs.len()
        )));
    }
    Ok(())
}

fn keyboard_input(
    key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY,
    release: bool,
) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: if release {
                    KEYEVENTF_KEYUP
                } else {
                    Default::default()
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/windows_input.rs"]
mod tests;
