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
        let monitors: Vec<super::WindowsMonitorBounds> = app
            .available_monitors()
            .unwrap_or_default()
            .iter()
            .map(super::WindowsMonitorBounds::from_monitor)
            .collect();
        super::logical_position_from_physical(&monitors, x, y, width, height)
    }

    pub(super) fn clamped_position(
        app: &tauri::AppHandle,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> (f64, f64) {
        let monitors: Vec<super::WindowsMonitorBounds> = app
            .available_monitors()
            .unwrap_or_default()
            .iter()
            .map(super::WindowsMonitorBounds::from_monitor)
            .collect();
        super::clamped_popover_position(&monitors, x, y, width, height)
    }
}

#[cfg(target_os = "windows")]
pub use windows::{start, stop};

/// 建窗后以实际外框校正位置；Windows 的无标题栏可调整窗口仍有系统尺寸边框。
/// 初始逻辑尺寸只表示内容区，不能用它假定 HWND 的外框同样大小。
pub(crate) fn constrain_native_popover(window: &tauri::WebviewWindow) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let monitor = window
            .current_monitor()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "无法确定翻译气泡所在显示器".to_string())?;
        let work = monitor.work_area();
        let position = window.outer_position().map_err(|error| error.to_string())?;
        let size = window.outer_size().map_err(|error| error.to_string())?;
        let left = i64::from(work.position.x);
        let top = i64::from(work.position.y);
        let max_x = (left + i64::from(work.size.width) - i64::from(size.width)).max(left);
        let max_y = (top + i64::from(work.size.height) - i64::from(size.height)).max(top);
        let x = i64::from(position.x).clamp(left, max_x) as i32;
        let y = i64::from(position.y).clamp(top, max_y) as i32;
        if (x, y) != (position.x, position.y) {
            // 已经是实际物理坐标，不能再次交给 LogicalPosition 乘以 DPI。
            window
                .set_position(tauri::PhysicalPosition::new(x, y))
                .map_err(|error| error.to_string())?;
        }
    }
    #[cfg(not(target_os = "windows"))]
    let _ = window;
    Ok(())
}

/// Windows 显示器物理矩形、物理工作区矩形与真实缩放因子描述。
///
/// Tauri 在 Windows 上报告的 `Monitor::position`、`Monitor::size` 与 `Monitor::work_area`
/// 均为物理像素，而 `WebviewWindowBuilder::position(x, y)` 接收逻辑坐标并在建窗时乘以
/// 目标显示器的 `scale_factor`。因此所有边界判断与夹取统一在对应显示器的逻辑坐标空间完成，
/// 避免把物理工作区坐标直接传给 Tauri 导致再次乘以 DPI。
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct WindowsMonitorBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub work_x: i32,
    pub work_y: i32,
    pub work_width: u32,
    pub work_height: u32,
    pub scale_factor: f64,
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
impl WindowsMonitorBounds {
    pub fn from_monitor(monitor: &tauri::Monitor) -> Self {
        let position = monitor.position();
        let size = monitor.size();
        let work_area = monitor.work_area();
        Self {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
            work_x: work_area.position.x,
            work_y: work_area.position.y,
            work_width: work_area.size.width,
            work_height: work_area.size.height,
            scale_factor: monitor.scale_factor(),
        }
    }

    pub fn is_valid(&self) -> bool {
        self.width > 0
            && self.height > 0
            && self.scale_factor.is_finite()
            && self.scale_factor > 0.0
    }

    /// 显示器完整物理边界 `(left, top, right, bottom)`。
    fn physical_monitor_bounds(&self) -> (f64, f64, f64, f64) {
        let left = f64::from(self.x);
        let top = f64::from(self.y);
        let right = left + f64::from(self.width);
        let bottom = top + f64::from(self.height);
        (left, top, right, bottom)
    }

    /// 显示器完整逻辑边界 `(left, top, right, bottom)`。
    pub fn logical_monitor_bounds(&self) -> (f64, f64, f64, f64) {
        let (left, top, right, bottom) = self.physical_monitor_bounds();
        let scale = self.scale_factor;
        (left / scale, top / scale, right / scale, bottom / scale)
    }

    /// 显示器有效逻辑工作区边界 `(left, top, right, bottom)`。
    ///
    /// 若系统报告的 `work_area` 尺寸为 0 或越出当前显示器范围，则裁剪到显示器范围内；
    /// 裁剪后仍为空时退回显示器完整范围，保证窗口坐标始终落在当前显示器内。
    pub fn logical_work_bounds(&self) -> (f64, f64, f64, f64) {
        let (mon_left, mon_top, mon_right, mon_bottom) = self.physical_monitor_bounds();
        let raw_left = f64::from(self.work_x);
        let raw_top = f64::from(self.work_y);
        let raw_right = raw_left + f64::from(self.work_width);
        let raw_bottom = raw_top + f64::from(self.work_height);

        let clamped_left = raw_left.clamp(mon_left, mon_right);
        let clamped_top = raw_top.clamp(mon_top, mon_bottom);
        let clamped_right = raw_right.clamp(mon_left, mon_right);
        let clamped_bottom = raw_bottom.clamp(mon_top, mon_bottom);

        let (work_left, work_top, work_right, work_bottom) = if self.work_width > 0
            && self.work_height > 0
            && clamped_right > clamped_left
            && clamped_bottom > clamped_top
        {
            (clamped_left, clamped_top, clamped_right, clamped_bottom)
        } else {
            (mon_left, mon_top, mon_right, mon_bottom)
        };

        let scale = self.scale_factor;
        (
            work_left / scale,
            work_top / scale,
            work_right / scale,
            work_bottom / scale,
        )
    }

    /// 将逻辑坐标 `(x, y)` 按给定逻辑窗口尺寸 `(width, height)` 夹取在当前显示器逻辑工作区内。
    pub fn clamp_logical(&self, x: f64, y: f64, width: f64, height: f64) -> (f64, f64) {
        let (work_left, work_top, work_right, work_bottom) = self.logical_work_bounds();
        let clean_width = if width.is_nan() || width < 0.0 {
            0.0
        } else {
            width
        };
        let clean_height = if height.is_nan() || height < 0.0 {
            0.0
        } else {
            height
        };
        let max_x = (work_right - clean_width).max(work_left);
        let max_y = (work_bottom - clean_height).max(work_top);
        let clamped_x = if x.is_finite() {
            x.clamp(work_left, max_x)
        } else {
            work_left
        };
        let clamped_y = if y.is_finite() {
            y.clamp(work_top, max_y)
        } else {
            work_top
        };
        (clamped_x, clamped_y)
    }
}

/// 将 Windows 物理鼠标坐标 `(x, y)` 转换为逻辑坐标并约束在命中的显示器逻辑工作区内。
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn logical_position_from_physical(
    monitors: &[WindowsMonitorBounds],
    x: i32,
    y: i32,
    width: f64,
    height: f64,
) -> (f64, f64) {
    let px = f64::from(x);
    let py = f64::from(y);
    let matched = monitors
        .iter()
        .filter(|m| m.is_valid())
        .find(|m| {
            let (left, top, right, bottom) = m.physical_monitor_bounds();
            px >= left && px < right && py >= top && py < bottom
        })
        .or_else(|| {
            monitors.iter().filter(|m| m.is_valid()).find(|m| {
                let (left, top, right, bottom) = m.physical_monitor_bounds();
                px >= left && px <= right && py >= top && py <= bottom
            })
        });

    if let Some(monitor) = matched {
        let scale = monitor.scale_factor;
        let logical_x = (px + 8.0) / scale;
        let logical_y = (py + 8.0) / scale;
        return monitor.clamp_logical(logical_x, logical_y, width, height);
    }
    (px, py)
}

/// 将已经是逻辑坐标的翻译气泡位置 `(x, y)` 约束在所属显示器的逻辑工作区内。
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn clamped_popover_position(
    monitors: &[WindowsMonitorBounds],
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> (f64, f64) {
    if !x.is_finite() || !y.is_finite() {
        if let Some(first) = monitors.iter().find(|m| m.is_valid()) {
            return first.clamp_logical(x, y, width, height);
        }
        return (
            if x.is_finite() { x } else { 0.0 },
            if y.is_finite() { y } else { 0.0 },
        );
    }

    let matched = monitors
        .iter()
        .filter(|m| m.is_valid())
        .find(|m| {
            let (left, top, right, bottom) = m.logical_monitor_bounds();
            x >= left && x < right && y >= top && y < bottom
        })
        .or_else(|| {
            monitors.iter().filter(|m| m.is_valid()).find(|m| {
                let (left, top, right, bottom) = m.logical_monitor_bounds();
                x >= left && x <= right && y >= top && y <= bottom
            })
        });

    if let Some(monitor) = matched {
        return monitor.clamp_logical(x, y, width, height);
    }
    (x, y)
}

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
    #[cfg(target_os = "windows")]
    {
        windows::clamped_position(app, x, y, width, height)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (app, width, height);
        (x, y)
    }
}

#[cfg(test)]
#[path = "../tests/unit/translator_mouse.rs"]
mod tests;
