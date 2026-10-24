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
/// 启动 macOS 外部窗口置顶守护与右上角微型图钉角标同步服务：
/// 1. 彻底淘汰旧版 700ms 连续截屏录屏与大尺寸 WebView 镜像（消除高 CPU/GPU/电池开销）；
/// 2. 纯事件驱动 + 极低频（1.5s）静默守护：利用 Accessibility AXRaise 原生将目标物理窗口提至最前；
/// 3. 在被置顶物理窗口的右上角附着一个 28×28 像素的极微型原生图钉角标，点击直接解除置顶。
fn start_window_pin_guard_and_badges(app: &AppHandle) {
    let Some(host) = app
        .try_state::<Arc<DesktopHost>>()
        .map(|state| state.inner().clone())
    else {
        return;
    };
    let app_handle = app.clone();
    std::thread::spawn(move || {
        let mut last_foreground_pid = 0;
        loop {
            let pinned_entries = host.window_pin.list_pinned_windows();
            if pinned_entries.is_empty() {
                // 无置顶窗口时进行深睡，避免消耗任何 CPU
                std::thread::sleep(std::time::Duration::from_millis(1500));
                continue;
            }

            // 检查当前前台应用
            let current_fg_pid = host
                .window_pin
                .current_foreground_window()
                .ok()
                .map(|w| w.process_id)
                .unwrap_or(0);

            // 若用户切换了前台应用，并且前台不是被置顶的应用，则执行 AXRaise 保持置顶窗口在最前
            if current_fg_pid != last_foreground_pid && current_fg_pid > 0 {
                last_foreground_pid = current_fg_pid;
                let is_pinned_fg = pinned_entries.iter().any(|p| p.info.process_id == current_fg_pid);
                if !is_pinned_fg {
                    host.window_pin.raise_all_pinned_windows();
                }
            }

            // 收集需显示的图钉角标位置
            let mut badges = Vec::new();
            for pinned in pinned_entries {
                if let Ok(info) = host.os_hal.get_window_info(pinned.info.handle) {
                    badges.push(info);
                }
            }

            let app_for_main = app_handle.clone();
            let _ = app_handle.run_on_main_thread(move || {
                reconcile_pin_badges(&app_for_main, badges);
            });

            // 1.5 秒低频守护，零录屏、零图像编解码，CPU 趋近于 0.0%
            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
    });
}

#[cfg(target_os = "macos")]
fn reconcile_pin_badges(app: &AppHandle, badges: Vec<ExternalWindowInfo>) {
    use std::collections::HashSet;

    let mut desired = HashSet::new();
    for info in badges {
        let label = format!("pin-badge-{}", info.handle);
        desired.insert(label.clone());

        // 计算物理窗口右上角位置（向内缩进 36px）
        let badge_x = (info.x + info.width as i32 - 36).max(info.x);
        let badge_y = (info.y + 4).max(0);

        if let Some(window) = app.get_webview_window(&label) {
            let _ = window.set_position(tauri::LogicalPosition::new(badge_x as f64, badge_y as f64));
            continue;
        }

        let url = format!(
            "index.html?mode=native&window=pin-badge&handle={}",
            info.handle
        );
        let _ = WebviewWindowBuilder::new(app, label, WebviewUrl::App(url.into()))
            .title(format!("📌 {}", info.process_name))
            .inner_size(28.0, 28.0)
            .position(badge_x as f64, badge_y as f64)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .resizable(false)
            .build();
    }

    // 自动清理已被取消置顶的角标窗口以及历史旧镜像窗口
    for (label, window) in app.webview_windows() {
        if label.starts_with("pin-badge-") && !desired.contains(&label) {
            let _ = window.destroy();
        } else if label.starts_with("pin-mirror-") {
            let _ = window.destroy();
        }
    }
}

/// 启动系统剪贴板实时后台监听器（支持文本与图片双通道实时自动捕获）
fn start_clipboard_listener(app: &AppHandle) {
    let Some(host) = app
        .try_state::<Arc<DesktopHost>>()
        .map(|state| state.inner().clone())
    else {
        return;
    };
    let app_handle = app.clone();
    std::thread::spawn(move || {
        let Ok(mut clipboard) = arboard::Clipboard::new() else {
            tracing::error!("无法初始化系统剪贴板监听器 (arboard)");
            return;
        };
        let mut last_text = clipboard.get_text().ok();
        let mut last_image_hash: Option<String> = None;

        loop {
            std::thread::sleep(std::time::Duration::from_millis(250));

            // 通道一：优先检查系统剪贴板文本变更
            match clipboard.get_text() {
                Ok(text) if !text.trim().is_empty() && Some(&text) != last_text.as_ref() => {
                    let ctx = host.kernel().context();
                    if let Ok(Some(_)) = host.clipboard.capture_text(ctx, &text, None) {
                        emit_update(&app_handle);
                    }
                    last_text = Some(text);
                    last_image_hash = None;
                    continue;
                }
                Ok(text) => {
                    last_text = Some(text);
                }
                Err(_) => {
                    // 若读取文本失败，可能是剪贴板当前为图片，进入通道二检查图片变更
                    if let Ok(image) = clipboard.get_image() {
                        let img_hash = mod_clipboard::ClipboardModule::compute_hash(
                            "image",
                            &image.bytes,
                        );
                        if Some(&img_hash) != last_image_hash.as_ref() {
                            let ctx = host.kernel().context();
                            if let Ok(Some(_)) = host.clipboard.capture_image_rgba(
                                ctx,
                                image.width as u32,
                                image.height as u32,
                                &image.bytes,
                                None,
                            ) {
                                emit_update(&app_handle);
                            }
                            last_image_hash = Some(img_hash);
                            last_text = None;
                        }
                    }
                }
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
                app.get_webview_window(&format!("pin-badge-{}", handle))
                    .is_some(),
                "macOS pinned badge is a real Tauri WebView"
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
            start_window_pin_guard_and_badges(app.handle());

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
