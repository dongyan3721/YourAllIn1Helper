//! File-save feedback survives the originating capture window being destroyed.
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub(crate) fn notify_saved(app: &AppHandle, path: &str) {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let serial = NEXT.fetch_add(1, Ordering::Relaxed);
    // Percent-encode UTF-8 so paths cannot become URL parameters or HTML.
    let encoded: String = path
        .as_bytes()
        .iter()
        .map(|byte| format!("%{byte:02X}"))
        .collect();
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if crate::tauri_app::is_exiting(&handle) {
            return;
        }
        for (label, window) in handle.webview_windows() {
            if label.starts_with("save-toast-") {
                let _ = window.destroy();
            }
        }
        let label = format!("save-toast-{serial}");
        let url = format!("index.html?mode=native&window={label}&path={encoded}");
        let result = WebviewWindowBuilder::new(&handle, &label, WebviewUrl::App(url.into()))
            .title("Saved")
            .inner_size(480.0, 150.0)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .focusable(false)
            .resizable(false)
            .visible(false)
            .build();
        let Ok(window) = result else {
            return;
        };
        if let Some(monitor) = handle
            .cursor_position()
            .ok()
            .and_then(|p| handle.monitor_from_point(p.x, p.y).ok().flatten())
        {
            let scale = monitor.scale_factor();
            let pos = monitor.position();
            let size = monitor.size();
            let _ = window.set_position(tauri::PhysicalPosition::new(
                pos.x + size.width as i32 - (500.0 * scale) as i32,
                pos.y + size.height as i32 - (230.0 * scale) as i32,
            ));
        }
        let _ = window.set_ignore_cursor_events(true);
        let _ = crate::tauri_app::show_passive_window(&window);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(7));
            if let Some(window) = handle.get_webview_window(&label) {
                let _ = window.destroy();
            }
        });
    });
}
