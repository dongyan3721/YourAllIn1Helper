//! 密码本自己的 Windows 会话/电源观察者。
//!
//! 只订阅当前会话的 `WM_WTSSESSION_CHANGE`，以及发给本隐藏窗口的睡眠通知。
//! 锁屏、断开、注销和睡眠会锁定密码本；解锁和唤醒不会自动打开它。
//! 这里不读取输入内容，也不弹出认证框。
use crate::{error::VaultError, LockReason, VaultResult, VaultService};
use std::{
    collections::HashMap,
    sync::{mpsc, Arc, Mutex},
    thread::{self, JoinHandle},
    time::Duration,
};
use windows::Win32::{
    Foundation::{GetLastError, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
    Graphics::Gdi::HBRUSH,
    System::{
        LibraryLoader::GetModuleHandleW,
        Power::{
            RegisterSuspendResumeNotification, UnregisterSuspendResumeNotification, HPOWERNOTIFY,
        },
        RemoteDesktop::{
            WTSRegisterSessionNotification, WTSUnRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION,
        },
    },
    UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, PostMessageW,
        PostQuitMessage, RegisterClassW, SendMessageW, TranslateMessage, DEVICE_NOTIFY_WINDOW_HANDLE,
        HCURSOR, HICON, MSG, PBT_APMSUSPEND, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND,
        WM_CLOSE, WM_DESTROY, WM_ENDSESSION, WM_POWERBROADCAST, WM_QUERYENDSESSION,
        WM_WTSSESSION_CHANGE, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
        WTS_CONSOLE_DISCONNECT, WTS_REMOTE_DISCONNECT, WTS_SESSION_LOCK, WTS_SESSION_LOGOFF,
    },
};

const CLASS_NAME: windows::core::PCWSTR = windows::core::w!("AllInOneVaultSessionObserver");
const ERROR_CLASS_ALREADY_EXISTS: u32 = 1410;

struct Slot {
    service: Arc<VaultService>,
    power: isize,
}

static SLOTS: Mutex<Option<HashMap<isize, Slot>>> = Mutex::new(None);

fn slots() -> std::sync::MutexGuard<'static, Option<HashMap<isize, Slot>>> {
    SLOTS.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// 官方会话事件。解锁、连接和未知值不锁定，更不会解锁。
pub(crate) fn session_lock_reason(event: u32) -> Option<LockReason> {
    match event {
        WTS_SESSION_LOCK | WTS_CONSOLE_DISCONNECT | WTS_REMOTE_DISCONNECT | WTS_SESSION_LOGOFF => {
            Some(LockReason::ScreenLock)
        }
        _ => None,
    }
}

/// 睡眠才锁定。恢复通知保持原状态，避免唤醒时自动打开密码本。
pub(crate) fn power_lock_reason(event: u32) -> Option<LockReason> {
    match event {
        PBT_APMSUSPEND => Some(LockReason::Sleep),
        PBT_APMRESUMESUSPEND | PBT_APMRESUMEAUTOMATIC => None,
        _ => None,
    }
}

pub(crate) fn window_message_reason(message: u32, wparam: usize) -> Option<LockReason> {
    match message {
        WM_WTSSESSION_CHANGE => session_lock_reason(wparam as u32),
        WM_POWERBROADCAST => power_lock_reason(wparam as u32),
        WM_QUERYENDSESSION => Some(LockReason::ScreenLock),
        WM_ENDSESSION if wparam != 0 => Some(LockReason::ScreenLock),
        _ => None,
    }
}

pub struct NativeVaultObserver {
    hwnd: Option<HWND>,
    thread: Option<JoinHandle<()>>,
}

impl NativeVaultObserver {
    pub fn start(service: Arc<VaultService>) -> VaultResult<Self> {
        let (tx, rx) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("password-vault-session".to_string())
            .spawn(move || session_thread(service, tx))
            .map_err(|_| {
                VaultError::FeatureDisabled("windows session observer thread did not start")
            })?;
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(raw)) => Ok(Self {
                hwnd: Some(HWND(raw as *mut core::ffi::c_void)),
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                tracing::error!("windows session observer did not become ready");
                std::mem::forget(thread);
                Err(VaultError::FeatureDisabled(
                    "windows session observer timed out",
                ))
            }
        }
    }

    pub(crate) fn hwnd(&self) -> Option<HWND> {
        self.hwnd
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if let Some(hwnd) = self.hwnd.take() {
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for NativeVaultObserver {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn session_thread(service: Arc<VaultService>, tx: mpsc::Sender<VaultResult<isize>>) {
    match setup(service) {
        Ok(hwnd) => {
            let _ = tx.send(Ok(hwnd));
            run_loop();
        }
        Err(error) => {
            let _ = tx.send(Err(error));
        }
    }
}

fn setup(service: Arc<VaultService>) -> VaultResult<isize> {
    register_class()?;
    let instance = module_handle()?;
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            CLASS_NAME,
            CLASS_NAME,
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(HINSTANCE(instance.0)),
            None,
        )
    }
    .map_err(|error| {
        tracing::warn!(code = error.code().0, "vault session window was not created");
        VaultError::FeatureDisabled("windows session observer could not create its window")
    })?;
    let raw = hwnd.0 as isize;
    slots()
        .get_or_insert_with(HashMap::new)
        .insert(raw, Slot { service, power: 0 });
    if let Err(error) = register_notifications(hwnd) {
        cleanup_window(hwnd);
        return Err(error);
    }
    Ok(raw)
}

fn module_handle() -> VaultResult<windows::Win32::Foundation::HMODULE> {
    unsafe { GetModuleHandleW(None) }.map_err(|_| {
        VaultError::FeatureDisabled("windows session observer could not read the module handle")
    })
}

fn register_class() -> VaultResult<()> {
    let instance = module_handle()?;
    let class = WNDCLASSW {
        lpfnWndProc: Some(wnd_proc),
        hInstance: HINSTANCE(instance.0),
        hIcon: HICON::default(),
        hCursor: HCURSOR::default(),
        hbrBackground: HBRUSH::default(),
        lpszClassName: CLASS_NAME,
        ..WNDCLASSW::default()
    };
    let atom = unsafe { RegisterClassW(&class) };
    if atom == 0 {
        let error = unsafe { GetLastError() };
        if error.0 != ERROR_CLASS_ALREADY_EXISTS {
            tracing::warn!(code = error.0, "vault session window class was rejected");
            return Err(VaultError::FeatureDisabled(
                "windows session observer could not register its window class",
            ));
        }
    }
    Ok(())
}

fn register_notifications(hwnd: HWND) -> VaultResult<()> {
    unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) }.map_err(|error| {
        tracing::warn!(code = error.code().0, "WTSRegisterSessionNotification failed");
        VaultError::FeatureDisabled("windows session notifications are unavailable")
    })?;
    match unsafe {
        RegisterSuspendResumeNotification(HANDLE(hwnd.0), DEVICE_NOTIFY_WINDOW_HANDLE)
    } {
        Ok(handle) => {
            if let Some(slot) = slots()
                .as_mut()
                .and_then(|map| map.get_mut(&(hwnd.0 as isize)))
            {
                slot.power = handle.0;
            }
            Ok(())
        }
        Err(error) => {
            tracing::warn!(
                code = error.code().0,
                "RegisterSuspendResumeNotification failed"
            );
            unsafe {
                let _ = WTSUnRegisterSessionNotification(hwnd);
            }
            Err(VaultError::FeatureDisabled(
                "windows sleep notifications are unavailable",
            ))
        }
    }
}

fn cleanup_window(hwnd: HWND) {
    unregister(hwnd);
    slots()
        .as_mut()
        .map(|map| map.remove(&(hwnd.0 as isize)));
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

fn unregister(hwnd: HWND) {
    let power = {
        let mut guard = slots();
        guard.as_mut().and_then(|map| map.get_mut(&(hwnd.0 as isize))).map(|slot| {
            let power = slot.power;
            slot.power = 0;
            power
        })
    };
    if let Some(power) = power.filter(|power| *power != 0) {
        unsafe {
            let _ = UnregisterSuspendResumeNotification(HPOWERNOTIFY(power));
        }
    }
    unsafe {
        let _ = WTSUnRegisterSessionNotification(hwnd);
    }
}

fn run_loop() {
    let mut message = MSG::default();
    loop {
        let status = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if status.0 <= 0 {
            break;
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if let Some(reason) = window_message_reason(message, wparam.0) {
        let service = slots().as_ref().and_then(|map| {
            map.get(&(hwnd.0 as isize))
                .map(|slot| Arc::clone(&slot.service))
        });
        if let Some(service) = service {
            service.lock(reason);
        }
    }
    match message {
        WM_CLOSE => {
            unregister(hwnd);
            let _ = unsafe { DestroyWindow(hwnd) };
            LRESULT(0)
        }
        WM_DESTROY => {
            slots()
                .as_mut()
                .map(|map| map.remove(&(hwnd.0 as isize)));
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        WM_QUERYENDSESSION | WM_POWERBROADCAST => LRESULT(1),
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// 测试只向自己的隐藏窗口发送官方消息，不锁定真实工作站，也不弹出认证框。
#[cfg(test)]
pub(crate) fn inject_for_test(hwnd: HWND, message: u32, wparam: usize) {
    unsafe {
        SendMessageW(hwnd, message, Some(WPARAM(wparam)), Some(LPARAM(0)));
    }
}
