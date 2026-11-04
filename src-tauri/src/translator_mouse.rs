//! macOS 鼠标选择入口。原生回调只采集手势；AX 查询与窗口操作在串行后台线程执行。
#[cfg(target_os = "macos")]
mod macos {
    use crate::DesktopHost;
    use objc2::{rc::Retained, runtime::AnyObject};
    use objc2_app_kit::{NSEvent, NSEventMask, NSEventType, NSWorkspace};
    use std::{
        cell::RefCell,
        sync::{
            atomic::{AtomicU64, Ordering},
            mpsc, Arc,
        },
        time::Duration,
    };
    use tauri::Manager;

    thread_local! {
        static MONITOR: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    }
    enum Gesture {
        Clear,
        Selection {
            generation: u64,
            pid: i32,
            x: f64,
            y: f64,
        },
    }

    pub fn start(app: &tauri::AppHandle) {
        let (tx, rx) = mpsc::channel();
        let generation = Arc::new(AtomicU64::new(0));
        let current = generation.clone();
        let worker_app = app.clone();
        std::thread::spawn(move || loop {
            let event = rx.recv_timeout(Duration::from_secs(8));
            let Some(host) = worker_app.try_state::<Arc<DesktopHost>>() else {
                break;
            };
            let result = host.kernel().registry().with_enabled("translator", || {
                let ctx = host.kernel().context();
                match event {
                    Ok(Gesture::Selection {
                        generation: id,
                        pid,
                        x,
                        y,
                    }) => {
                        if !host.translator.config().mouse_selection_enabled {
                            return Ok(());
                        }
                        // 等目标控件处理完 mouseUp；期间不改变前台焦点。
                        std::thread::sleep(Duration::from_millis(100));
                        if current.load(Ordering::SeqCst) != id || foreground_pid() != pid {
                            return Ok(());
                        }
                        let selection = host.translator.extract_selected_text();
                        if current.load(Ordering::SeqCst) != id || foreground_pid() != pid {
                            return Ok(());
                        }
                        // 自动入口无权限/不支持时不反复弹权限窗口，快捷键入口仍有明确指引。
                        if let Ok(Some(selection)) = selection {
                            let (x, y) = clamped_position(&worker_app, x, y, 20.0, 20.0);
                            host.translator
                                .offer_mouse_selection(ctx, selection, x, y)?;
                        }
                    }
                    _ => {
                        host.translator.dismiss_mouse_selection(ctx)?;
                    }
                }
                Ok(())
            });
            if let Err(error) = result {
                if !matches!(error, app_core::CoreError::ModuleDisabled(_)) {
                    tracing::warn!(%error, "Mouse translation entry failed");
                }
            }
            if matches!(event, Err(mpsc::RecvTimeoutError::Disconnected)) {
                break;
            }
        });
        let callback_app = app.clone();
        let down = RefCell::new(None::<(f64, f64, i32)>);
        let block = block2::RcBlock::new(move |event: std::ptr::NonNull<NSEvent>| {
            let event = unsafe { event.as_ref() };
            let point = NSEvent::mouseLocation();
            match event.r#type() {
                NSEventType::LeftMouseDown => {
                    generation.fetch_add(1, Ordering::SeqCst);
                    *down.borrow_mut() = Some((point.x, point.y, foreground_pid()));
                    let _ = tx.send(Gesture::Clear);
                }
                NSEventType::LeftMouseUp => {
                    let Some((x, y, pid)) = down.borrow_mut().take() else {
                        return;
                    };
                    if !is_selection_drag((x, y, pid), (point.x, point.y, foreground_pid())) {
                        return;
                    }
                    // AppKit 全局坐标左下原点，Tauri 逻辑坐标左上原点。
                    let top = callback_app
                        .primary_monitor()
                        .ok()
                        .flatten()
                        .map(|m| m.size().height as f64 / m.scale_factor())
                        .unwrap_or(0.0);
                    let _ = tx.send(Gesture::Selection {
                        generation: generation.load(Ordering::SeqCst),
                        pid,
                        x: point.x + 8.0,
                        y: top - point.y + 8.0,
                    });
                }
                _ => {}
            }
        });
        let monitor = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
            NSEventMask::LeftMouseDown | NSEventMask::LeftMouseUp,
            &block,
        );
        MONITOR.with(|slot| *slot.borrow_mut() = monitor);
    }

    pub(super) fn is_selection_drag(start: (f64, f64, i32), end: (f64, f64, i32)) -> bool {
        start.2 > 0
            && start.2 != std::process::id() as i32
            && start.2 == end.2
            && (end.0 - start.0).hypot(end.1 - start.1) >= 6.0
    }

    fn foreground_pid() -> i32 {
        NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .map(|a| a.processIdentifier())
            .unwrap_or(0)
    }

    pub(super) fn clamped_position(
        app: &tauri::AppHandle,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> (f64, f64) {
        for monitor in app.available_monitors().unwrap_or_default() {
            let scale = monitor.scale_factor();
            let p = monitor.position();
            let s = monitor.size();
            let left = p.x as f64 / scale;
            let top = p.y as f64 / scale;
            let right = left + s.width as f64 / scale;
            let bottom = top + s.height as f64 / scale;
            if x >= left && x < right && y >= top && y < bottom {
                return (
                    x.clamp(left, (right - width).max(left)),
                    y.clamp(top, (bottom - height).max(top)),
                );
            }
        }
        (x, y)
    }

    pub fn stop() {
        MONITOR.with(|slot| {
            if let Some(monitor) = slot.borrow_mut().take() {
                unsafe {
                    NSEvent::removeMonitor(&monitor);
                }
            }
        });
    }
}
#[cfg(target_os = "macos")]
pub use macos::{start, stop};

#[cfg(target_os = "windows")]
mod windows {
    use crate::DesktopHost;
    use ::windows::Win32::{
        Foundation::{LPARAM, LRESULT, WPARAM},
        System::Threading::GetCurrentThreadId,
        UI::WindowsAndMessaging::{
            CallNextHookEx, DispatchMessageW, GetForegroundWindow, GetMessageW,
            GetWindowThreadProcessId, PeekMessageW, SetWindowsHookExW, TranslateMessage,
            UnhookWindowsHookEx, MSG, MSLLHOOKSTRUCT, PM_NOREMOVE, WH_MOUSE_LL, WM_LBUTTONDOWN,
            WM_LBUTTONUP, WM_QUIT,
        },
    };
    use std::{
        sync::{
            atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
            mpsc::{self, Receiver, Sender},
            Mutex, OnceLock,
        },
        thread::JoinHandle,
        time::Duration,
    };
    use tauri::Manager;

    static EVENT_SENDER: OnceLock<Mutex<Option<Sender<MouseEvent>>>> = OnceLock::new();
    static HOOK_THREAD_ID: AtomicU32 = AtomicU32::new(0);
    static STARTED: AtomicBool = AtomicBool::new(false);
    static GENERATION: AtomicU64 = AtomicU64::new(0);
    static HOOK_THREAD: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);
    static WORKER_THREAD: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

    #[derive(Clone, Copy)]
    enum MouseEvent {
        Down { x: i32, y: i32, pid: u32 },
        Up { x: i32, y: i32, pid: u32 },
    }

    pub fn start(app: &tauri::AppHandle) {
        if STARTED.swap(true, Ordering::SeqCst) {
            return;
        }

        let (sender, receiver) = mpsc::channel();
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let hook_thread = std::thread::spawn(move || {
            let thread_id = unsafe { GetCurrentThreadId() };
            HOOK_THREAD_ID.store(thread_id, Ordering::SeqCst);
            let mut message = MSG::default();
            unsafe {
                let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
            }
            *EVENT_SENDER
                .get_or_init(|| Mutex::new(None))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(sender);

            let hook = match unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), None, 0) } {
                Ok(hook) => hook,
                Err(error) => {
                    let _ = ready_sender.send(Err(error.to_string()));
                    EVENT_SENDER
                        .get_or_init(|| Mutex::new(None))
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .take();
                    HOOK_THREAD_ID.store(0, Ordering::SeqCst);
                    return;
                }
            };
            let _ = ready_sender.send(Ok(()));
            loop {
                let status = unsafe { GetMessageW(&mut message, None, 0, 0) };
                if status.0 <= 0 {
                    if status.0 < 0 {
                        tracing::warn!("Windows selection hook message loop failed");
                    }
                    break;
                }
                unsafe {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            if let Err(error) = unsafe { UnhookWindowsHookEx(hook) } {
                tracing::warn!(%error, "Failed to remove Windows mouse hook");
            }
            EVENT_SENDER
                .get_or_init(|| Mutex::new(None))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take();
            HOOK_THREAD_ID.store(0, Ordering::SeqCst);
        });

        match ready_receiver.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                STARTED.store(false, Ordering::SeqCst);
                tracing::warn!(%error, "Failed to install Windows selection mouse hook");
                let _ = hook_thread.join();
                return;
            }
            Err(error) => {
                STARTED.store(false, Ordering::SeqCst);
                tracing::warn!(%error, "Timed out installing Windows selection mouse hook");
                let thread_id = HOOK_THREAD_ID.load(Ordering::SeqCst);
                if thread_id != 0 {
                    let _ = unsafe {
                        ::windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                            thread_id,
                            WM_QUIT,
                            WPARAM(0),
                            LPARAM(0),
                        )
                    };
                }
                let _ = hook_thread.join();
                EVENT_SENDER
                    .get_or_init(|| Mutex::new(None))
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .take();
                return;
            }
        }

        *HOOK_THREAD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(hook_thread);
        let worker_app = app.clone();
        let worker_thread = std::thread::spawn(move || run_selection_worker(worker_app, receiver));
        *WORKER_THREAD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(worker_thread);
    }

    pub fn stop() {
        if !STARTED.swap(false, Ordering::SeqCst) {
            return;
        }
        let thread_id = HOOK_THREAD_ID.load(Ordering::SeqCst);
        if thread_id != 0 {
            if let Err(error) = unsafe {
                ::windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                    thread_id,
                    WM_QUIT,
                    WPARAM(0),
                    LPARAM(0),
                )
            } {
                tracing::warn!(%error, "Failed to stop Windows selection mouse hook");
            }
        }
        if let Some(thread) = HOOK_THREAD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            let _ = thread.join();
        }
        if let Some(sender) = EVENT_SENDER
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            drop(sender);
        }
        if let Some(thread) = WORKER_THREAD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            let _ = thread.join();
        }
    }

    unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 && (wparam.0 as u32 == WM_LBUTTONDOWN || wparam.0 as u32 == WM_LBUTTONUP) {
            let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
            let mut pid = 0;
            let foreground = unsafe { GetForegroundWindow() };
            unsafe {
                GetWindowThreadProcessId(foreground, Some(&mut pid));
            }
            if let Some(sender) = EVENT_SENDER
                .get()
                .and_then(|slot| slot.lock().ok())
                .and_then(|sender| sender.clone())
            {
                let event = if wparam.0 as u32 == WM_LBUTTONDOWN {
                    MouseEvent::Down {
                        x: info.pt.x,
                        y: info.pt.y,
                        pid,
                    }
                } else {
                    MouseEvent::Up {
                        x: info.pt.x,
                        y: info.pt.y,
                        pid,
                    }
                };
                let _ = sender.send(event);
            }
        }
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    fn run_selection_worker(app: tauri::AppHandle, receiver: Receiver<MouseEvent>) {
        let mut down = None;
        while let Ok(event) = receiver.recv() {
            let Some(host) = app.try_state::<std::sync::Arc<DesktopHost>>() else {
                break;
            };
            let result = host.kernel().registry().with_enabled("translator", || {
                let ctx = host.kernel().context();
                match event {
                    MouseEvent::Down { x, y, pid } => {
                        GENERATION.fetch_add(1, Ordering::SeqCst);
                        down = Some((x, y, pid));
                        host.translator.dismiss_mouse_selection(ctx)?;
                    }
                    MouseEvent::Up { x, y, pid } => {
                        let Some((start_x, start_y, start_pid)) = down.take() else {
                            return Ok(());
                        };
                        if !host.translator.config().mouse_selection_enabled {
                            return Ok(());
                        }
                        let id = GENERATION.load(Ordering::SeqCst);
                        if start_pid == 0
                            || start_pid == std::process::id()
                            || start_pid != pid
                            || (i64::from(x) - i64::from(start_x)).pow(2)
                                + (i64::from(y) - i64::from(start_y)).pow(2)
                                < 36
                        {
                            return Ok(());
                        }

                        std::thread::sleep(Duration::from_millis(100));
                        if GENERATION.load(Ordering::SeqCst) != id || foreground_pid() != start_pid
                        {
                            return Ok(());
                        }
                        let Ok(Some(selection)) = host.translator.extract_selected_text() else {
                            return Ok(());
                        };
                        if GENERATION.load(Ordering::SeqCst) != id || foreground_pid() != start_pid
                        {
                            return Ok(());
                        }
                        let (x, y) = logical_position(&app, x, y, 20.0, 20.0);
                        host.translator
                            .offer_mouse_selection(ctx, selection, x, y)?;
                    }
                }
                Ok(())
            });
            if let Err(error) = result {
                if !matches!(error, app_core::CoreError::ModuleDisabled(_)) {
                    tracing::warn!(%error, "Windows mouse translation entry failed");
                }
            }
        }
    }

    fn foreground_pid() -> u32 {
        let foreground = unsafe { GetForegroundWindow() };
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(foreground, Some(&mut pid));
        }
        pid
    }

    fn logical_position(
        app: &tauri::AppHandle,
        x: i32,
        y: i32,
        width: f64,
        height: f64,
    ) -> (f64, f64) {
        for monitor in app.available_monitors().unwrap_or_default() {
            let scale = monitor.scale_factor();
            let position = monitor.position();
            let size = monitor.size();
            let left = position.x as f64;
            let top = position.y as f64;
            let right = left + size.width as f64;
            let bottom = top + size.height as f64;
            if x as f64 >= left && (x as f64) < right && y as f64 >= top && (y as f64) < bottom {
                let logical_left = left / scale;
                let logical_top = top / scale;
                let logical_width = size.width as f64 / scale;
                let logical_height = size.height as f64 / scale;
                return (
                    (logical_left + (x as f64 - left + 8.0) / scale).clamp(
                        logical_left,
                        logical_left + (logical_width - width).max(0.0),
                    ),
                    (logical_top + (y as f64 - top + 8.0) / scale).clamp(
                        logical_top,
                        logical_top + (logical_height - height).max(0.0),
                    ),
                );
            }
        }
        (x as f64, y as f64)
    }
}

#[cfg(target_os = "windows")]
pub use windows::{start, stop};

/// 翻译气泡按本次尺寸留在当前显示器内。
pub fn popover_position(
    app: &tauri::AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> (f64, f64) {
    #[cfg(target_os = "macos")]
    {
        macos::clamped_position(app, x, y, width, height)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, width, height);
        (x, y)
    }
}

#[cfg(all(test, target_os = "macos"))]
#[path = "../tests/unit/translator_mouse.rs"]
mod tests;
