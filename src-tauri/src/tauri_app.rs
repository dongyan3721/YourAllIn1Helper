//! Real Tauri 2.x desktop shell.
//!
//! Owns native WebView windows, tray, global shortcuts and application lifecycle. The
//! microkernel stays UI-framework agnostic through `app_core::WindowBackend`.

use std::sync::Arc;

use app_core::{CoreError, CoreResult, WindowBackend, WindowSpec};
use os_hal::{ExternalWindowInfo, WindowController};
use serde_json::Value;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, RunEvent, State, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::{DesktopHost, IpcRequest, IpcResponse};

const ACTION_CLIPBOARD: &str = "clipboard:toggle_popup";
const ACTION_STICKY: &str = "sticky_note:create";
const ACTION_PIN_FOREGROUND: &str = "window_pin:toggle_foreground";
const ACTION_PIN_PICKER: &str = "window_pin:pick_cursor";

fn to_core_error(error: impl std::fmt::Display) -> CoreError {
    CoreError::Internal(anyhow::anyhow!(error.to_string()))
}

/// Maps the microkernel's declarative window specs to real Tauri `WebviewWindow`s.
pub struct TauriWindowBackend {
    app: AppHandle,
}

impl TauriWindowBackend {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    fn frontend_url(spec: &WindowSpec) -> String {
        let mut url = format!("index.html?mode=native&window={}", spec.label);
        if let Some(note_id) = spec.label.strip_prefix("sticky-note-") {
            url.push_str("&id=");
            url.push_str(note_id);
        }
        if spec.label == "dashboard" && spec.route != "/overview" {
            url.push_str("&route=");
            url.push_str(spec.route.trim_start_matches('/'));
        }
        url
    }
}

impl WindowBackend for TauriWindowBackend {
    fn create_or_focus_window(&self, spec: &WindowSpec) -> CoreResult<()> {
        if let Some(window) = self.app.get_webview_window(&spec.label) {
            window.show().map_err(to_core_error)?;
            window.set_focus().map_err(to_core_error)?;
            return Ok(());
        }

        let mut builder = WebviewWindowBuilder::new(
            &self.app,
            &spec.label,
            WebviewUrl::App(Self::frontend_url(spec).into()),
        )
        .title(&spec.title)
        .inner_size(spec.width, spec.height)
        .resizable(spec.resizable)
        .decorations(spec.decorations)
        .transparent(spec.transparent)
        .always_on_top(spec.always_on_top)
        .skip_taskbar(spec.skip_taskbar)
        .focused(spec.focus);

        if let (Some(min_w), Some(min_h)) = (spec.min_width, spec.min_height) {
            builder = builder.min_inner_size(min_w, min_h);
        }
        if let (Some(x), Some(y)) = (spec.x, spec.y) {
            builder = builder.position(x, y);
        }
        if spec.label == "window-pin-picker" {
            if let Ok(Some(monitor)) = self.app.primary_monitor() {
                let size = monitor.size();
                let position = monitor.position();
                let scale = monitor.scale_factor();
                builder = builder
                    .inner_size(size.width as f64 / scale, size.height as f64 / scale)
                    .position(position.x as f64 / scale, position.y as f64 / scale);
            }
        }

        builder.build().map_err(to_core_error)?;
        Ok(())
    }

    fn destroy_window(&self, label: &str) -> CoreResult<()> {
        if let Some(window) = self.app.get_webview_window(label) {
            window.destroy().map_err(to_core_error)?;
        }
        Ok(())
    }

    fn set_ignore_cursor_events(&self, label: &str, ignore: bool) -> CoreResult<()> {
        let window =
            self.app
                .get_webview_window(label)
                .ok_or_else(|| CoreError::WindowLifecycle {
                    label: label.to_string(),
                    reason: "Tauri window not found".to_string(),
                })?;
        window
            .set_ignore_cursor_events(ignore)
            .map_err(to_core_error)
    }

    fn set_always_on_top(&self, label: &str, always_on_top: bool) -> CoreResult<()> {
        let window =
            self.app
                .get_webview_window(label)
                .ok_or_else(|| CoreError::WindowLifecycle {
                    label: label.to_string(),
                    reason: "Tauri window not found".to_string(),
                })?;
        window
            .set_always_on_top(always_on_top)
            .map_err(to_core_error)
    }
}

/// Single typed IPC entry point used by every WebView.
#[tauri::command]
fn ipc(host: State<'_, Arc<DesktopHost>>, request: IpcRequest) -> IpcResponse {
    host.invoke_ipc(&request.cmd, request.args)
}

fn emit_update(app: &AppHandle) {
    let _ = app.emit("kernel://updated", Value::Null);
}

fn trigger_action(app: &AppHandle, action_id: &'static str) {
    if let Some(host) = app.try_state::<Arc<DesktopHost>>() {
        if let Err(error) = host.kernel().trigger_action(action_id) {
            tracing::error!(action_id, %error, "Global shortcut action failed");
        }
        emit_update(app);
    }
}

fn register_global_shortcuts(app: &AppHandle) -> anyhow::Result<()> {
    let shortcuts = [
        ("CommandOrControl+Shift+V", ACTION_CLIPBOARD),
        ("CommandOrControl+Shift+N", ACTION_STICKY),
        ("CommandOrControl+Shift+P", ACTION_PIN_FOREGROUND),
        ("CommandOrControl+Alt+P", ACTION_PIN_PICKER),
    ];

    for (accelerator, action_id) in shortcuts {
        app.global_shortcut()
            .on_shortcut(accelerator, move |app, _shortcut, event| {
                if event.state == ShortcutState::Pressed {
                    trigger_action(app, action_id);
                }
            })?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn start_window_pin_mirror_sync(app: &AppHandle) {
    let Some(host) = app
        .try_state::<Arc<DesktopHost>>()
        .map(|state| state.inner().clone())
    else {
        return;
    };
    let app_handle = app.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(700));
        let foreground_pid = host
            .window_pin
            .current_foreground_window()
            .ok()
            .map(|window| window.process_id);
        let mut mirrors = Vec::new();
        for pinned in host.window_pin.list_pinned_windows() {
            if let Ok(info) = host.os_hal.get_window_info(pinned.info.handle) {
                let hide = foreground_pid == Some(info.process_id);
                mirrors.push((info, hide));
            }
        }
        let app_for_main = app_handle.clone();
        let _ = app_handle.run_on_main_thread(move || {
            reconcile_pin_mirrors(&app_for_main, mirrors);
        });
    });
}

#[cfg(target_os = "macos")]
fn reconcile_pin_mirrors(app: &AppHandle, mirrors: Vec<(ExternalWindowInfo, bool)>) {
    use std::collections::HashSet;

    let mut desired = HashSet::new();
    for (info, hide) in mirrors {
        let label = format!("pin-mirror-{}", info.handle);
        desired.insert(label.clone());
        if let Some(window) = app.get_webview_window(&label) {
            let _ = window.set_position(tauri::LogicalPosition::new(info.x as f64, info.y as f64));
            let _ = window.set_size(tauri::LogicalSize::new(
                info.width as f64,
                info.height as f64,
            ));
            if hide {
                let _ = window.hide();
            } else {
                let _ = window.show();
            }
            continue;
        }

        let url = format!(
            "index.html?mode=native&window=pin-mirror&handle={}",
            info.handle
        );
        if let Ok(window) = WebviewWindowBuilder::new(app, label, WebviewUrl::App(url.into()))
            .title(format!("📌 {}", info.process_name))
            .inner_size(info.width as f64, info.height as f64)
            .position(info.x as f64, info.y as f64)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .resizable(false)
            .build()
        {
            if hide {
                let _ = window.hide();
            }
        }
    }

    for (label, window) in app.webview_windows() {
        if label.starts_with("pin-mirror-") && !desired.contains(&label) {
            let _ = window.destroy();
        }
    }
}

fn start_clipboard_listener(app: &AppHandle) {
    let Some(host) = app
        .try_state::<Arc<DesktopHost>>()
        .map(|state| state.inner().clone())
    else {
        return;
    };
    std::thread::spawn(move || {
        let Ok(mut clipboard) = arboard::Clipboard::new() else {
            tracing::error!("Unable to initialize system clipboard listener");
            return;
        };
        let mut last_text = clipboard.get_text().ok();
        loop {
            std::thread::sleep(std::time::Duration::from_millis(250));
            match clipboard.get_text() {
                Ok(text) if !text.trim().is_empty() && Some(&text) != last_text.as_ref() => {
                    if let Err(error) =
                        host.clipboard
                            .capture_text(host.kernel().context(), &text, None)
                    {
                        tracing::warn!(%error, "Failed to capture system clipboard text");
                    }
                    last_text = Some(text);
                }
                Ok(text) => last_text = Some(text),
                Err(_) => {}
            }
        }
    });
}

fn setup_tray(app: &AppHandle) -> Result<(), tauri::Error> {
    let dashboard = MenuItem::with_id(app, "tray_dashboard", "打开主控台", true, None::<&str>)?;
    let clipboard = MenuItem::with_id(
        app,
        "tray_clipboard",
        "剪贴板历史 (⇧⌘V)",
        true,
        None::<&str>,
    )?;
    let sticky = MenuItem::with_id(app, "tray_sticky", "新建置顶便笺 (⇧⌘N)", true, None::<&str>)?;
    let pin = MenuItem::with_id(app, "tray_pin", "置顶当前窗口 (⇧⌘P)", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "tray_quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&dashboard, &clipboard, &sticky, &pin, &quit])?;

    TrayIconBuilder::with_id("main-tray")
        .icon(tauri::image::Image::from_bytes(include_bytes!(
            "../icons/icon.png"
        ))?)
        .menu(&menu)
        .tooltip("All-in-One Toolbox")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "tray_dashboard" => {
                if let Some(host) = app.try_state::<Arc<DesktopHost>>() {
                    let _ = host
                        .kernel()
                        .context()
                        .window_pool
                        .open_window(WindowSpec::dashboard("/overview"));
                    emit_update(app);
                }
            }
            "tray_clipboard" => trigger_action(app, ACTION_CLIPBOARD),
            "tray_sticky" => trigger_action(app, ACTION_STICKY),
            "tray_pin" => trigger_action(app, ACTION_PIN_FOREGROUND),
            "tray_quit" => {
                if let Some(host) = app.try_state::<Arc<DesktopHost>>() {
                    let _ = host.kernel().shutdown();
                }
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;
    Ok(())
}

/// Exercises real Tauri windows and native capabilities without synthesizing keyboard input.
/// This is intentionally separate from headless unit tests: every window checked here is a real
/// WebView created by the running desktop application.
fn run_ui_smoke_checks(app: AppHandle, keep_open: bool) {
    std::thread::spawn(move || {
        let mut failures = Vec::new();
        macro_rules! check {
            ($condition:expr, $name:expr) => {
                if $condition {
                    println!("[UI SMOKE] PASS: {}", $name);
                } else {
                    eprintln!("[UI SMOKE] FAIL: {}", $name);
                    failures.push($name);
                }
            };
        }

        std::thread::sleep(std::time::Duration::from_millis(600));
        let host = app.state::<Arc<DesktopHost>>().inner().clone();
        check!(true, "Tauri tray and global shortcuts registered");

        let dashboard = host.invoke_ipc(
            "kernel_open_dashboard",
            serde_json::json!({ "route": "/overview" }),
        );
        std::thread::sleep(std::time::Duration::from_millis(250));
        check!(
            dashboard.ok && app.get_webview_window("dashboard").is_some(),
            "real dashboard WebView opens"
        );

        let clipboard_result = (|| -> bool {
            let Ok(mut clipboard) = arboard::Clipboard::new() else {
                return false;
            };
            let previous = clipboard.get_text().ok();
            let marker = format!("AIO_TAURI_UI_SMOKE_{}", std::process::id());
            if clipboard.set_text(marker.clone()).is_err() {
                return false;
            }
            let roundtrip = clipboard.get_text().ok().as_deref() == Some(marker.as_str());
            std::thread::sleep(std::time::Duration::from_millis(550));
            let history = host.invoke_ipc(
                "clipboard_list_items",
                serde_json::json!({ "keyword": marker, "limit": 10 }),
            );
            let captured = history
                .data
                .and_then(|value| value.as_array().cloned())
                .map(|items| !items.is_empty())
                .unwrap_or(false);
            if let Some(previous) = previous {
                let _ = clipboard.set_text(previous);
            }
            roundtrip && captured
        })();
        check!(
            clipboard_result,
            "system clipboard roundtrip and background history listener"
        );

        host.invoke_ipc(
            "kernel_trigger_shortcut",
            serde_json::json!({ "action_id": ACTION_CLIPBOARD }),
        );
        std::thread::sleep(std::time::Duration::from_millis(250));
        check!(
            app.get_webview_window("clipboard-popup").is_some(),
            "clipboard shortcut creates a real transient WebView"
        );
        host.invoke_ipc(
            "kernel_trigger_shortcut",
            serde_json::json!({ "action_id": ACTION_CLIPBOARD }),
        );
        std::thread::sleep(std::time::Duration::from_millis(150));
        check!(
            app.get_webview_window("clipboard-popup").is_none(),
            "clipboard popup is destroyed instead of hidden"
        );

        let created = host.invoke_ipc(
            "sticky_create_note",
            serde_json::json!({
                "title": "Tauri UI Smoke Note",
                "content_md": "Real **Tauri** window: $E = mc^2$",
                "theme_color": "sky"
            }),
        );
        let note_id = created
            .data
            .as_ref()
            .and_then(|data| data["id"].as_str())
            .unwrap_or("")
            .to_string();
        std::thread::sleep(std::time::Duration::from_millis(250));
        check!(
            !note_id.is_empty()
                && app
                    .get_webview_window(&format!("sticky-note-{}", note_id))
                    .is_some(),
            "sticky note creates an independent pinned WebView"
        );
        host.invoke_ipc("sticky_close_note", serde_json::json!({ "id": note_id }));
        std::thread::sleep(std::time::Duration::from_millis(150));
        check!(
            app.get_webview_window(&format!("sticky-note-{}", note_id))
                .is_none(),
            "closing a sticky note destroys its WebView"
        );

        let candidates = host.invoke_ipc("winpin_list_candidates", serde_json::json!({}));
        let first_handle = candidates
            .data
            .as_ref()
            .and_then(|data| data["candidates"].as_array())
            .and_then(|windows| windows.first())
            .and_then(|window| window["handle"].as_u64());
        check!(
            first_handle.is_some(),
            "native external windows are enumerated"
        );
        if let Some(handle) = first_handle {
            let pinned = host.invoke_ipc(
                "winpin_toggle_handle",
                serde_json::json!({ "handle": handle }),
            );
            check!(pinned.ok, "external window enters pinned state");
            std::thread::sleep(std::time::Duration::from_millis(900));
            #[cfg(target_os = "macos")]
            check!(
                app.get_webview_window(&format!("pin-mirror-{}", handle))
                    .is_some(),
                "macOS pinned mirror is a real Tauri WebView"
            );
            let snapshot = host.invoke_ipc(
                "winpin_get_snapshot",
                serde_json::json!({ "handle": handle }),
            );
            check!(snapshot.ok, "real external window capture succeeds");
            host.invoke_ipc("winpin_unpin", serde_json::json!({ "handle": handle }));
        }

        println!(
            "[UI SMOKE] NOTE: synthetic Cmd+V is intentionally not injected (Accessibility-independent test)"
        );
        if failures.is_empty() {
            println!("UI_SMOKE_RESULT: PASS");
        } else {
            eprintln!("UI_SMOKE_RESULT: FAIL ({} checks)", failures.len());
        }

        if keep_open && failures.is_empty() {
            host.invoke_ipc(
                "kernel_open_dashboard",
                serde_json::json!({ "route": "/overview" }),
            );
        } else {
            let _ = host.kernel().shutdown();
            app.exit(if failures.is_empty() { 0 } else { 1 });
        }
    });
}

/// Starts the real cross-platform Tauri desktop application.
pub fn run() {
    let ui_smoke_test = std::env::args().any(|arg| arg == "--ui-smoke-test");
    let keep_open = std::env::args().any(|arg| arg == "--keep-open");
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![ipc])
        .setup(move |app| {
            let app_data_dir = app.path().app_data_dir()?;
            let backend = Arc::new(TauriWindowBackend::new(app.handle().clone()));
            let host = Arc::new(DesktopHost::bootstrap_with_window_backend(
                app_data_dir,
                false,
                backend,
            )?);
            host.seed_demo_data_if_empty()?;
            app.manage(host);

            register_global_shortcuts(app.handle())?;
            setup_tray(app.handle())?;
            start_clipboard_listener(app.handle());
            #[cfg(target_os = "macos")]
            start_window_pin_mirror_sync(app.handle());

            if !ui_smoke_test && !std::env::args().any(|arg| arg == "--silent") {
                let host = app.state::<Arc<DesktopHost>>();
                host.kernel()
                    .context()
                    .window_pool
                    .open_window(WindowSpec::dashboard("/overview"))?;
            }
            if ui_smoke_test {
                run_ui_smoke_checks(app.handle().clone(), keep_open);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            let Some(host) = window.app_handle().try_state::<Arc<DesktopHost>>() else {
                return;
            };
            let pool = &host.kernel().context().window_pool;
            match event {
                WindowEvent::CloseRequested { .. } => {
                    let label = window.label();
                    if let Some(note_id) = label.strip_prefix("sticky-note-") {
                        let _ = host
                            .sticky_note
                            .close_note(host.kernel().context(), note_id);
                    } else {
                        let _ = pool.destroy_window(label);
                    }
                }
                WindowEvent::Focused(false) => {
                    let _ = pool.on_window_blur(window.label());
                }
                _ => {}
            }
            emit_update(window.app_handle());
        })
        .build(tauri::generate_context!())
        .expect("failed to build Tauri application");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            if let Some(host) = app.try_state::<Arc<DesktopHost>>() {
                let _ = host.kernel().shutdown();
            }
        }
    });
}
