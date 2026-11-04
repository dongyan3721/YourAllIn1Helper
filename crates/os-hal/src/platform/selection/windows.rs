//! Windows 平台原生无障碍（UIAutomation / TextPattern）选区提取适配器。

use crate::types::{HalError, HalResult, SelectionSource, SelectionTextResult};

const MAX_SELECTION_CHARS: i32 = 64_000;

pub fn foreground_process_id() -> Option<u32> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    let mut process_id = 0;
    unsafe {
        GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut process_id));
    }
    (process_id != 0).then_some(process_id)
}

/// 从当前键盘焦点控件提取 UIA TextPattern 选区，不激活或修改目标窗口。
pub fn extract_selected_text_windows(
    foreground_pid: Option<u32>,
    foreground_name: Option<String>,
) -> HalResult<Option<SelectionTextResult>> {
    #[cfg(target_os = "windows")]
    {
        std::thread::Builder::new()
            .name("windows-selection-extractor".into())
            .spawn(move || extract_on_worker(foreground_pid, foreground_name))
            .map_err(|error| {
                HalError::NativeCallFailed(format!("启动 UIAutomation 工作线程失败：{error}"))
            })?
            .join()
            .map_err(|_| HalError::NativeCallFailed("UIAutomation 工作线程意外终止".into()))?
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (foreground_pid, foreground_name);
        Err(HalError::Unsupported(
            "windows_selection_extraction_on_non_windows",
        ))
    }
}

#[cfg(target_os = "windows")]
fn extract_on_worker(
    foreground_pid: Option<u32>,
    foreground_name: Option<String>,
) -> HalResult<Option<SelectionTextResult>> {
    use windows::Win32::{
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_MULTITHREADED,
        },
        UI::Accessibility::{
            CUIAutomation, IUIAutomation, IUIAutomationTextPattern, UIA_TextPatternId,
        },
    };

    struct ComApartment;
    impl Drop for ComApartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(|error| {
                HalError::NativeCallFailed(format!("初始化 UIAutomation COM 失败：{error}"))
            })?;
    }
    let _apartment = ComApartment;

    let automation: IUIAutomation = unsafe {
        CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).map_err(|error| {
            HalError::NativeCallFailed(format!("创建 UIAutomation 客户端失败：{error}"))
        })?
    };
    let focused = unsafe { automation.GetFocusedElement() }.map_err(|error| {
        HalError::NativeCallFailed(format!("读取 UIAutomation 焦点控件失败：{error}"))
    })?;

    let process_id = unsafe { focused.CurrentProcessId() }.map_err(|error| {
        HalError::NativeCallFailed(format!("读取 UIAutomation 控件进程失败：{error}"))
    })?;
    if process_id <= 0
        || process_id == std::process::id() as i32
        || foreground_pid.is_some_and(|expected| expected != process_id as u32)
    {
        return Ok(None);
    }

    let pattern: IUIAutomationTextPattern =
        unsafe { focused.GetCurrentPatternAs(UIA_TextPatternId) }
            .map_err(|_| HalError::Unsupported("windows_uiautomation_text_pattern"))?;
    let ranges = unsafe { pattern.GetSelection() }
        .map_err(|_| HalError::Unsupported("windows_uiautomation_selection"))?;
    let count = unsafe { ranges.Length() }.map_err(|error| {
        HalError::NativeCallFailed(format!("读取 UIAutomation 选区数量失败：{error}"))
    })?;
    let mut selected = Vec::new();
    for index in 0..count.max(0) {
        let range = unsafe { ranges.GetElement(index) }.map_err(|error| {
            HalError::NativeCallFailed(format!("读取 UIAutomation 选区失败：{error}"))
        })?;
        let text = unsafe { range.GetText(MAX_SELECTION_CHARS) }.map_err(|error| {
            HalError::NativeCallFailed(format!("读取 UIAutomation 选区文本失败：{error}"))
        })?;
        let text = text.to_string();
        if !text.trim().is_empty() {
            selected.push(text);
        }
    }
    if selected.is_empty() {
        return Ok(None);
    }

    Ok(Some(SelectionTextResult {
        text: selected.join("\n"),
        source: SelectionSource::AccessibilityApi,
        source_app_name: foreground_name,
    }))
}
