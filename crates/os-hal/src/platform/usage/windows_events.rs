//! Windows 锁屏 / 息屏 / 睡眠事件观察者。
//!
//! 在专用后台线程中创建隐藏顶层窗口，注册会话通知（WTSRegisterSessionNotification）
//! 与电源通知（RegisterSuspendResumeNotification、RegisterPowerSettingNotification），
//! 监听 WM_WTSSESSION_CHANGE 与 WM_POWERBROADCAST 消息；
//! 观察者只更新原子状态，业务模块通过 ActiveAppWatcher 读取。
//! 停止时撤销所有通知、销毁窗口并等待线程退出，重复启停保持幂等。

use super::{UsageActivityEvent, UsageActivityState};
use crate::{HalError, HalResult};
use parking_lot::Mutex;
use std::ffi::c_void;
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use windows::core::{w, GUID};
use windows::Win32::Foundation::{GetLastError, HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Power::{
    RegisterPowerSettingNotification, RegisterSuspendResumeNotification,
    UnregisterPowerSettingNotification, UnregisterSuspendResumeNotification,
    POWERBROADCAST_SETTING,
};
use windows::Win32::System::RemoteDesktop::{
    WTSRegisterSessionNotification, WTSUnRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetWindowLongPtrW, PostMessageW, PostQuitMessage, RegisterClassExW, SetWindowLongPtrW,
    TranslateMessage, UnregisterClassW, CREATESTRUCTW, CW_USEDEFAULT, DEVICE_NOTIFY_WINDOW_HANDLE,
    GWLP_USERDATA, MSG, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND,
    PBT_POWERSETTINGCHANGE, WINDOW_EX_STYLE, WM_CLOSE, WM_NCDESTROY, WM_POWERBROADCAST,
    WM_WTSSESSION_CHANGE, WNDCLASSEXW, WS_OVERLAPPED, WTS_SESSION_LOCK, WTS_SESSION_LOGOFF,
    WTS_SESSION_UNLOCK,
};

/// 屏幕显示状态通知 GUID：Windows 10+ 会话显示单一权威源
const GUID_SESSION_DISPLAY_STATUS: GUID =
    GUID::from_u128(0x2b84c10b_ed39_4dc5_b3ce_e4ed38a3db32);
/// 控制台显示状态通知 GUID：旧版 Windows 回退源
const GUID_CONSOLE_DISPLAY_STATE: GUID =
    GUID::from_u128(0x25df14d1_1c6d_47fb_acb0_44b37494f0c4);

/// 传递给窗口过程的上下文指针
struct EventMonitorContext {
    state: Arc<UsageActivityState>,
}

/// 窗口过程回调：捕获会话改变与电源状态广播，更新状态机
unsafe extern "system" fn event_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        windows::Win32::UI::WindowsAndMessaging::WM_NCCREATE => {
            let cs = lparam.0 as *const CREATESTRUCTW;
            if !cs.is_null() {
                let context_ptr = (*cs).lpCreateParams as isize;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, context_ptr);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_WTSSESSION_CHANGE => {
            let context_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const EventMonitorContext;
            if !context_ptr.is_null() {
                let state = &(*context_ptr).state;
                let event_type = wparam.0 as u32;
                match event_type {
                    WTS_SESSION_LOCK => {
                        state.apply(UsageActivityEvent::ScreenLocked);
                    }
                    WTS_SESSION_UNLOCK => {
                        state.apply(UsageActivityEvent::ScreenUnlocked);
                    }
                    WTS_SESSION_LOGOFF => {
                        state.apply(UsageActivityEvent::ScreenLocked);
                    }
                    _ => {}
                }
            }
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            let context_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const EventMonitorContext;
            if !context_ptr.is_null() {
                let state = &(*context_ptr).state;
                let power_event = wparam.0 as u32;
                match power_event {
                    PBT_APMSUSPEND => {
                        state.apply(UsageActivityEvent::WillSleep);
                    }
                    PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => {
                        state.apply(UsageActivityEvent::DidWake);
                    }
                    PBT_POWERSETTINGCHANGE => {
                        let setting_ptr = lparam.0 as *const POWERBROADCAST_SETTING;
                        if !setting_ptr.is_null() {
                            let setting = &*setting_ptr;
                            let guid = setting.PowerSetting;
                            if (guid == GUID_SESSION_DISPLAY_STATUS
                                || guid == GUID_CONSOLE_DISPLAY_STATE)
                                && setting.DataLength >= 1
                            {
                                match setting.Data[0] {
                                    0 => state.apply(UsageActivityEvent::ScreensDidSleep),
                                    1 | 2 => state.apply(UsageActivityEvent::ScreensDidWake),
                                    _ => {}
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            // Win32 要求响应 WM_POWERBROADCAST 返回 TRUE (1)
            LRESULT(1)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            PostQuitMessage(0);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// 后台监视器工作线程逻辑
fn run_monitor(
    state: Arc<UsageActivityState>,
    ready_tx: mpsc::SyncSender<HalResult<isize>>,
) {
    let context = EventMonitorContext {
        state: Arc::clone(&state),
    };

    unsafe {
        let class_name = w!("AioUsageEventWindowClass");
        let mut wc: WNDCLASSEXW = std::mem::zeroed();
        wc.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
        wc.lpfnWndProc = Some(event_wnd_proc);
        wc.lpszClassName = class_name;

        // 注册窗口类，若类已存在（例如前次监控未彻底卸载或多实例）忽略 1410 (ERROR_CLASS_ALREADY_EXISTS)
        let atom = RegisterClassExW(&wc);
        if atom == 0 {
            let error_code = GetLastError().0;
            if error_code != 1410 {
                let _ = ready_tx.send(Err(HalError::NativeCallFailed(format!(
                    "RegisterClassExW failed with Win32 error {error_code}"
                ))));
                return;
            }
        }

        // 创建顶层隐藏窗口（不带 WS_VISIBLE，WS_OVERLAPPED 顶级窗口才能稳定接收系统广播通知）
        let hwnd_res = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            w!("AioUsageEventWindow"),
            WS_OVERLAPPED,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            0,
            0,
            None,
            None,
            None,
            Some(&context as *const _ as *const c_void),
        );

        let hwnd = match hwnd_res {
            Ok(h) => h,
            Err(e) => {
                let _ = UnregisterClassW(class_name, None);
                let _ = ready_tx.send(Err(HalError::NativeCallFailed(format!(
                    "CreateWindowExW failed: {e}"
                ))));
                return;
            }
        };

        // 1. 注册会话锁屏/解锁通知；失败时严禁吞错汇报 ready，必须清理资源并报错
        if let Err(err) = WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) {
            let _ = DestroyWindow(hwnd);
            let _ = UnregisterClassW(class_name, None);
            let _ = ready_tx.send(Err(HalError::NativeCallFailed(format!(
                "WTSRegisterSessionNotification failed: {err}"
            ))));
            return;
        }

        // 2. 注册电源睡眠/唤醒通知；失败时清理并报错
        let suspend_handle = match RegisterSuspendResumeNotification(
            HANDLE(hwnd.0),
            DEVICE_NOTIFY_WINDOW_HANDLE,
        ) {
            Ok(h) => h,
            Err(err) => {
                let _ = WTSUnRegisterSessionNotification(hwnd);
                let _ = DestroyWindow(hwnd);
                let _ = UnregisterClassW(class_name, None);
                let _ = ready_tx.send(Err(HalError::NativeCallFailed(format!(
                    "RegisterSuspendResumeNotification failed: {err}"
                ))));
                return;
            }
        };

        // 3. 注册屏幕显示状态通知：Windows 10+ 优先使用 GUID_SESSION_DISPLAY_STATUS 单一权威源，
        // 失败时回退到 GUID_CONSOLE_DISPLAY_STATE；若均失败则报错并回收资源，避免多源竞争写入
        let display_handle = match RegisterPowerSettingNotification(
            HANDLE(hwnd.0),
            &GUID_SESSION_DISPLAY_STATUS,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        ) {
            Ok(h) => h,
            Err(_) => match RegisterPowerSettingNotification(
                HANDLE(hwnd.0),
                &GUID_CONSOLE_DISPLAY_STATE,
                DEVICE_NOTIFY_WINDOW_HANDLE,
            ) {
                Ok(h) => h,
                Err(err) => {
                    let _ = UnregisterSuspendResumeNotification(suspend_handle);
                    let _ = WTSUnRegisterSessionNotification(hwnd);
                    let _ = DestroyWindow(hwnd);
                    let _ = UnregisterClassW(class_name, None);
                    let _ = ready_tx.send(Err(HalError::NativeCallFailed(format!(
                        "RegisterPowerSettingNotification display status failed: {err}"
                    ))));
                    return;
                }
            },
        };

        // 通知启动方就绪；若接收端已退出则回收全部资源
        if ready_tx.send(Ok(hwnd.0 as isize)).is_err() {
            let _ = UnregisterPowerSettingNotification(display_handle);
            let _ = UnregisterSuspendResumeNotification(suspend_handle);
            let _ = WTSUnRegisterSessionNotification(hwnd);
            let _ = DestroyWindow(hwnd);
            let _ = UnregisterClassW(class_name, None);
            return;
        }

        // Win32 标准消息循环：严格处理 GetMessageW 的三态返回值（>0 正常，0 正常退出 WM_QUIT，-1 出错）
        let mut msg = MSG::default();
        loop {
            let ret = GetMessageW(&mut msg, None, 0, 0);
            if ret.0 == 0 {
                // 收到 WM_QUIT，正常退出消息泵
                break;
            }
            if ret.0 == -1 {
                // GetMessageW 发生系统调用错误，安全跳出循环回收资源
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        // 消息循环退出后，在本线程依次注销通知、销毁窗口并注销窗口类
        let _ = UnregisterPowerSettingNotification(display_handle);
        let _ = UnregisterSuspendResumeNotification(suspend_handle);
        let _ = WTSUnRegisterSessionNotification(hwnd);
        let _ = DestroyWindow(hwnd);
        let _ = UnregisterClassW(class_name, None);
    }
}

/// 运行中的监视器实例封装
struct RunningInstance {
    hwnd: isize,
    thread: JoinHandle<()>,
}

/// Windows 锁屏/电源事件监视器
pub(super) struct WinUsageEventMonitor {
    state: Arc<UsageActivityState>,
    lifecycle: Mutex<Option<RunningInstance>>,
}

impl WinUsageEventMonitor {
    pub(super) fn new(state: Arc<UsageActivityState>) -> Self {
        Self {
            state,
            lifecycle: Mutex::new(None),
        }
    }

    /// 启动事件监视器；若已在运行则保持幂等直接返回 Ok(())
    pub(super) fn start(&self) -> HalResult<()> {
        let mut guard = self.lifecycle.lock();

        // 幂等性检查：若已有运行中的实例，检查线程是否仍在存活
        if let Some(instance) = guard.as_ref() {
            if !instance.thread.is_finished() {
                return Ok(());
            }
        }

        // 若前次线程已异常终止，清理旧句柄
        if let Some(instance) = guard.take() {
            let _ = instance.thread.join();
        }

        // 每次启动时重置事件状态，避免陈旧状态污染后续轮询
        self.state.reset();

        let (ready_tx, ready_rx) = mpsc::sync_channel::<HalResult<isize>>(1);
        let state = Arc::clone(&self.state);

        let handle = thread::Builder::new()
            .name("app-usage-events-win".to_string())
            .spawn(move || run_monitor(state, ready_tx))
            .map_err(|error| {
                HalError::NativeCallFailed(format!(
                    "failed to spawn Windows usage event thread: {error}"
                ))
            })?;

        match ready_rx.recv() {
            Ok(Ok(hwnd_val)) => {
                *guard = Some(RunningInstance {
                    hwnd: hwnd_val,
                    thread: handle,
                });
                Ok(())
            }
            Ok(Err(error)) => {
                let _ = handle.join();
                Err(error)
            }
            Err(_) => {
                let _ = handle.join();
                Err(HalError::NativeCallFailed(
                    "Windows usage event thread exited before ready".to_string(),
                ))
            }
        }
    }

    /// 停止事件监视器，向隐藏窗口投递 WM_CLOSE 并等待线程退出；未启动时安全无操作
    pub(super) fn stop(&self) {
        let mut guard = self.lifecycle.lock();
        if let Some(instance) = guard.take() {
            unsafe {
                let hwnd = HWND(instance.hwnd as *mut c_void);
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            let _ = instance.thread.join();
        }
        // 停止监视器后清理陈旧状态，保证后续严格轮询不被历史锁屏事件遮蔽
        self.state.reset();
    }
}

impl Drop for WinUsageEventMonitor {
    fn drop(&mut self) {
        self.stop();
    }
}
