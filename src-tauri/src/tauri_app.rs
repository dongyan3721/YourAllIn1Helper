//! Real Tauri 2.x desktop shell.
//!
//! Owns native WebView windows, tray, global shortcuts and application lifecycle. The
//! microkernel stays UI-framework agnostic through `app_core::WindowBackend`.

use std::sync::Arc;

use app_core::{CoreError, CoreResult, WindowBackend, WindowSpec};
use os_hal::ExternalWindowInfo;
use serde_json::Value;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, RunEvent, State, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::script_env::AioScriptEnvironment;
use crate::script_events::TauriScriptEventSink;
use crate::{DesktopHost, IpcRequest, IpcResponse};

const ACTION_CLIPBOARD: &str = "clipboard:toggle_popup";
const ACTION_SCREEN_INK: &str = "screen_ink:toggle_session";
const ACTION_STICKY: &str = "sticky_note:create";
const ACTION_PIN_FOREGROUND: &str = "window_pin:toggle_foreground";

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
            if spec.label == "dashboard" {
                self.app
                    .emit("dashboard://navigate", spec.route.clone())
                    .map_err(to_core_error)?;
            }
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
        if spec.label == "window-pin-picker" || spec.label == "screen-ink-canvas" {
            if let Ok(Some(monitor)) = self.app.primary_monitor() {
                let size = monitor.size();
                let position = monitor.position();
                let scale = monitor.scale_factor();
                builder = builder
                    .inner_size(size.width as f64 / scale, size.height as f64 / scale)
                    .position(position.x as f64 / scale, position.y as f64 / scale);
            }
        }
        if spec.label == "screen-ink-toolbar" {
            if let Ok(Some(monitor)) = self.app.primary_monitor() {
                let size = monitor.size();
                let position = monitor.position();
                let scale = monitor.scale_factor();
                let mon_w = size.width as f64 / scale;
                let mon_x = position.x as f64 / scale;
                let mon_y = position.y as f64 / scale;
                let x = mon_x + (mon_w - spec.width) / 2.0;
                let y = mon_y + 32.0;
                builder = builder.position(x, y);
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
fn ipc(app: AppHandle, host: State<'_, Arc<DesktopHost>>, request: IpcRequest) -> IpcResponse {
    let module_id = request
        .args
        .get("module_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let previous = module_id
        .as_deref()
        .map(|id| host.kernel().registry().is_enabled(id));
    let response = host.invoke_ipc(&request.cmd, request.args);
    if response.ok && request.cmd == "kernel_set_module_enabled" {
        if let Err(error) = register_global_shortcuts(&app) {
            if let (Some(id), Some(enabled)) = (module_id.as_deref(), previous) {
                let _ = host.kernel().set_module_enabled(id, enabled);
            }
            let _ = register_global_shortcuts(&app);
            emit_update(&app);
            return IpcResponse::err(format!("更新系统快捷键失败，已尝试还原模块状态：{}", error));
        }
        let _ = update_tray_menu(&app);
        if module_id.as_deref() == Some("window_pin")
            && !host.kernel().registry().is_enabled("window_pin")
        {
            for (label, window) in app.webview_windows() {
                if label.starts_with("pin-badge-") || label.starts_with("pin-mirror-") {
                    let _ = window.destroy();
                }
            }
        }
        if module_id.as_deref() == Some("screen_ink")
            && !host.kernel().registry().is_enabled("screen_ink")
        {
            for label in ["screen-ink-canvas", "screen-ink-toolbar"] {
                if let Some(window) = app.get_webview_window(label) {
                    let _ = window.destroy();
                }
            }
        }
        emit_update(&app);
    } else if response.ok && request.cmd == "kernel_update_general_config" {
        // 当用户切换多语言时，同步刷新系统托盘菜单多语言文案并通知各窗口
        let _ = update_tray_menu(&app);
        emit_update(&app);
    } else if response.ok && (request.cmd == "script_save" || request.cmd == "script_delete") {
        // 脚本 CRUD 可能改变单脚本快捷键与托盘快捷脚本，必须复用现有生命周期同步。
        let _ = register_global_shortcuts(&app);
        let _ = update_tray_menu(&app);
        emit_update(&app);
    }
    response
}

fn emit_update(app: &AppHandle) {
    let _ = app.emit("kernel://updated", Value::Null);
}

fn trigger_action(app: &AppHandle, action_id: &str) {
    if let Some(host) = app.try_state::<Arc<DesktopHost>>() {
        if let Err(error) = host.kernel().trigger_action(action_id) {
            tracing::error!(action_id, %error, "Global shortcut action failed");
        }
        emit_update(app);
    }
}

fn register_global_shortcuts(app: &AppHandle) -> anyhow::Result<()> {
    app.global_shortcut().unregister_all()?;
    let host = app.state::<Arc<DesktopHost>>();
    for shortcut in host.kernel().context().shortcut_registry.list_all() {
        let action_id = shortcut.spec.action_id;
        app.global_shortcut().on_shortcut(
            shortcut.physical_accelerator.as_str(),
            move |app, _, event| {
                if event.state == ShortcutState::Pressed {
                    trigger_action(app, &action_id);
                }
            },
        )?;
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
        let mut had_pinned_windows = false;
        loop {
            let pinned_entries = host.window_pin.list_pinned_windows();
            if pinned_entries.is_empty() {
                if had_pinned_windows {
                    // 当置顶列表归零变空时，确保在主线程执行一次 reconcile_pin_badges 清理所有残余角标窗口
                    had_pinned_windows = false;
                    let app_for_main = app_handle.clone();
                    let _ = app_handle.run_on_main_thread(move || {
                        reconcile_pin_badges(&app_for_main, vec![]);
                    });
                }
                // 无置顶窗口时进行深睡，避免消耗任何 CPU
                std::thread::sleep(std::time::Duration::from_millis(1500));
                continue;
            }

            had_pinned_windows = true;

            // 检查当前前台应用
            let current_fg_pid = host
                .window_pin
                .current_foreground_window()
                .ok()
                .map(|w| w.process_id)
                .unwrap_or(0);

            // 若用户切换了前台应用，并且前台不是被置顶的应用，则执行保持置顶动作
            if current_fg_pid != last_foreground_pid && current_fg_pid > 0 {
                last_foreground_pid = current_fg_pid;
                let is_pinned_fg = pinned_entries
                    .iter()
                    .any(|p| p.info.process_id == current_fg_pid);
                if !is_pinned_fg {
                    host.window_pin.raise_all_pinned_windows();
                }
            }

            // 收集需显示的图钉角标位置
            let mut badges = Vec::new();
            for pinned in pinned_entries {
                match host
                    .os_hal
                    .window_controller
                    .get_window_info(pinned.info.handle)
                {
                    Ok(info) => badges.push(info),
                    Err(_) => {
                        // 目标外部物理窗口已被用户在外部关闭，自动从置顶列表中解钉并清理
                        let _ = host.window_pin.unpin_window(pinned.info.handle);
                    }
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
            let _ =
                window.set_position(tauri::LogicalPosition::new(badge_x as f64, badge_y as f64));
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
        let mut clipboard: Option<arboard::Clipboard> = None;
        let mut last_text: Option<String> = None;
        let mut last_image_hash: Option<String> = None;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let result = host.kernel().registry().with_enabled("clipboard", || {
                if !host.kernel().context().config.read().clipboard.enabled {
                    clipboard = None;
                    return Ok(());
                }
                if clipboard.is_none() {
                    let mut opened = arboard::Clipboard::new().map_err(to_core_error)?;
                    last_text = opened.get_text().ok();
                    last_image_hash = None;
                    clipboard = Some(opened);
                    return Ok(());
                }
                let clipboard = clipboard.as_mut().expect("initialized above");
                // 通道一：优先检查系统剪贴板文本变更
                match clipboard.get_text() {
                    Ok(text) if !text.trim().is_empty() && Some(&text) != last_text.as_ref() => {
                        let ctx = host.kernel().context();
                        if let Ok(Some(_)) = host.clipboard.capture_text(ctx, &text, None) {
                            emit_update(&app_handle);
                        }
                        last_text = Some(text);
                        last_image_hash = None;
                        return Ok(());
                    }
                    Ok(text) => {
                        last_text = Some(text);
                    }
                    Err(_) => {
                        // 若读取文本失败，可能是剪贴板当前为图片，进入通道二检查图片变更
                        if let Ok(image) = clipboard.get_image() {
                            let img_hash =
                                mod_clipboard::ClipboardModule::compute_hash("image", &image.bytes);
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
                Ok(())
            });
            if result.is_err() {
                clipboard = None;
                last_text = None;
                last_image_hash = None;
            }
        }
    });
}

fn build_tray_menu(app: &AppHandle) -> Result<Menu<tauri::Wry>, tauri::Error> {
    let host = app.state::<Arc<DesktopHost>>();
    let lang = host
        .kernel()
        .context()
        .config
        .read()
        .general
        .language
        .to_ascii_lowercase();

    let (lbl_dash, lbl_clip, lbl_ink, lbl_sticky, lbl_pin, lbl_quit) = if lang.starts_with("en") {
        (
            "Open Dashboard",
            "Clipboard History (⇧⌘V)",
            "Screen Annotation (⇧⌘D)",
            "New Sticky Note (⇧⌘N)",
            "Pin Active Window (⇧⌘P)",
            "Quit",
        )
    } else if lang.starts_with("ja") || lang.starts_with("jp") {
        (
            "ダッシュボードを開く",
            "クリップボード履歴 (⇧⌘V)",
            "画面注釈 (⇧⌘D)",
            "新規付箋ノート (⇧⌘N)",
            "アクティブウィンドウを固定 (⇧⌘P)",
            "終了",
        )
    } else {
        (
            "打开主控台",
            "剪贴板历史 (⇧⌘V)",
            "屏幕注释 (⇧⌘D)",
            "新建置顶便笺 (⇧⌘N)",
            "置顶当前窗口 (⇧⌘P)",
            "退出",
        )
    };

    let dashboard = MenuItem::with_id(app, "tray_dashboard", lbl_dash, true, None::<&str>)?;
    let mut quick_scripts: Vec<(String, String)> = Vec::new();
    if host.kernel().registry().is_enabled("script_runner") {
        for script in host
            .script_runner
            .list_scripts(host.kernel().context())
            .unwrap_or_default()
        {
            if script.show_in_tray {
                quick_scripts.push((format!("tray_script_{}", script.id), script.name.clone()));
            }
        }
    }
    let clipboard = MenuItem::with_id(
        app,
        "tray_clipboard",
        lbl_clip,
        host.kernel().registry().is_enabled("clipboard"),
        None::<&str>,
    )?;
    let ink = MenuItem::with_id(
        app,
        "tray_ink",
        lbl_ink,
        host.kernel().registry().is_enabled("screen_ink"),
        None::<&str>,
    )?;
    let sticky = MenuItem::with_id(
        app,
        "tray_sticky",
        lbl_sticky,
        host.kernel().registry().is_enabled("sticky_note"),
        None::<&str>,
    )?;
    let pin = MenuItem::with_id(
        app,
        "tray_pin",
        lbl_pin,
        host.kernel().registry().is_enabled("window_pin")
            && host.os_hal.window_controller.supports_external_topmost(),
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "tray_quit", lbl_quit, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&dashboard, &clipboard, &ink, &sticky, &pin])?;
    for (id, label) in quick_scripts {
        let item = MenuItem::with_id(app, id, label, true, None::<&str>)?;
        menu.append(&item)?;
    }
    menu.append(&quit)?;
    Ok(menu)
}

fn update_tray_menu(app: &AppHandle) -> Result<(), tauri::Error> {
    if let Some(tray) = app.tray_by_id("main-tray") {
        tray.set_menu(Some(build_tray_menu(app)?))?;
    }
    Ok(())
}

fn setup_tray(app: &AppHandle) -> Result<(), tauri::Error> {
    let menu = build_tray_menu(app)?;

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
            "tray_ink" => trigger_action(app, ACTION_SCREEN_INK),
            "tray_sticky" => trigger_action(app, ACTION_STICKY),
            "tray_pin" => trigger_action(app, ACTION_PIN_FOREGROUND),
            "tray_quit" => {
                if let Some(host) = app.try_state::<Arc<DesktopHost>>() {
                    let _ = host.kernel().shutdown();
                }
                app.exit(0);
            }
            id if id.starts_with("tray_script_") => {
                if let Some(host) = app.try_state::<Arc<DesktopHost>>() {
                    if let Ok(script_id) = id.trim_start_matches("tray_script_").parse::<i64>() {
                        if let Err(error) = host.script_runner.execute(
                            host.kernel().context(),
                            script_id,
                            Vec::new(),
                        ) {
                            tracing::error!(script_id, %error, "Tray script execution failed");
                        }
                        emit_update(app);
                    }
                }
            }
            _ => {}
        })
        .build(app)?;
    Ok(())
}

#[path = "../tests/support/gui_test.rs"]
mod gui_test;
#[path = "../tests/support/ui_smoke.rs"]
mod ui_smoke;

/// Starts the real cross-platform Tauri desktop application.
pub fn run() {
    let ui_smoke_test = std::env::args().any(|arg| arg == "--ui-smoke-test");
    let keep_open = std::env::args().any(|arg| arg == "--keep-open");
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![ipc])
        .setup(move |app| {
            // 人工 GUI 回归可使用独立的数据目录，避免改动日常使用记录。
            let app_data_dir = if let Some(path) = gui_test::data_dir() {
                path
            } else if ui_smoke_test {
                std::env::temp_dir().join(format!("all-in-one-ui-smoke-{}", std::process::id()))
            } else {
                app.path().app_data_dir()?
            };
            let backend = Arc::new(TauriWindowBackend::new(app.handle().clone()));
            let native_hal = os_hal::NativeOsHal::new().into_bundle();
            let script_environment = Arc::new(AioScriptEnvironment::new(
                native_hal.selection_extractor.clone(),
            ));
            let script_events = Arc::new(TauriScriptEventSink::new(app.handle().clone()));
            let host = Arc::new(DesktopHost::bootstrap_with_adapters_and_scripting(
                app_data_dir,
                false,
                backend,
                native_hal,
                script_environment,
                script_events,
            )?);
            let event_app = app.handle().clone();
            host.media_proc.set_event_sink(Some(Arc::new(move |job| {
                let _ = event_app.emit(
                    "media://job-progress",
                    serde_json::json!({
                        "job_id": job.id,
                        "progress": job.progress,
                        "speed": job.speed,
                        "status": job.status,
                        "job": job,
                    }),
                );
            })));
            app.manage(host);

            register_global_shortcuts(app.handle())?;
            setup_tray(app.handle())?;
            start_clipboard_listener(app.handle());
            #[cfg(target_os = "macos")]
            if app
                .state::<Arc<DesktopHost>>()
                .os_hal
                .window_controller
                .supports_external_topmost()
            {
                start_window_pin_guard_and_badges(app.handle());
            }

            if !ui_smoke_test && !std::env::args().any(|arg| arg == "--silent") {
                let host = app.state::<Arc<DesktopHost>>();
                host.kernel()
                    .context()
                    .window_pool
                    .open_window(WindowSpec::dashboard("/overview"))?;
            }
            if ui_smoke_test {
                ui_smoke::run_ui_smoke_checks(app.handle().clone(), keep_open);
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
