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
