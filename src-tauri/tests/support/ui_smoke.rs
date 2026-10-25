//! 原生桌面烟测：使用临时数据与本程序窗口，不修改系统剪贴板或外部窗口。
use super::*;
// Tauri 的 destroy 会异步提交到主线程，必须观察实际窗口移除后再断言。
fn wait_until(mut condition: impl FnMut() -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !condition() {
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    true
}

/// 读取真实 AppKit 关系和窗口顺序，防止“两个窗口都存在”掩盖工具栏被画布遮住。
#[cfg(target_os = "macos")]
fn check_ink_toolbar_stacking(app: &AppHandle) -> anyhow::Result<()> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let result = (|| -> anyhow::Result<()> {
            use objc2::MainThreadMarker;
            use objc2_app_kit::{NSWindow, NSWindowNumberListOptions};

            let mtm = MainThreadMarker::new().unwrap();
            let canvas = handle.get_webview_window("screen-ink-canvas").unwrap();
            let toolbar = handle.get_webview_window("screen-ink-toolbar").unwrap();
            // SAFETY: Tauri 拥有两个存活窗口；仅在 AppKit 主线程借用原生指针。
            let canvas: &NSWindow = unsafe { &*canvas.ns_window()?.cast::<NSWindow>() };
            let toolbar: &NSWindow = unsafe { &*toolbar.ns_window()?.cast::<NSWindow>() };
            anyhow::ensure!(
                toolbar.parentWindow().is_some_and(|parent| std::ptr::eq(&*parent, canvas)),
                "screen ink toolbar must be a native child of canvas"
            );
            anyhow::ensure!(toolbar.level() > canvas.level(), "toolbar window level");
            let ordered = NSWindow::windowNumbersWithOptions(NSWindowNumberListOptions::empty(), mtm)
                .ok_or_else(|| anyhow::anyhow!("native window ordering unavailable"))?;
            let numbers: Vec<_> = ordered.iter().map(|number| number.integerValue()).collect();
            let toolbar_index = numbers.iter().position(|number| *number == toolbar.windowNumber());
            let canvas_index = numbers.iter().position(|number| *number == canvas.windowNumber());
            anyhow::ensure!(
                matches!((toolbar_index, canvas_index), (Some(toolbar), Some(canvas)) if toolbar < canvas),
                "toolbar must remain above canvas in native z-order: toolbar={}, canvas={}, ordered={numbers:?}",
                toolbar.windowNumber(), canvas.windowNumber()
            );
            Ok(())
        })();
        let _ = sender.send(result);
    })?;
    receiver.recv_timeout(std::time::Duration::from_secs(5))??;
    Ok(())
}

/// 读取遮罩的真实 AppKit 层级，确认位于菜单栏之上，而不只是「窗口存在」。
#[cfg(target_os = "macos")]
fn overlay_native_level(app: &AppHandle) -> anyhow::Result<isize> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let result = (|| -> anyhow::Result<isize> {
            use objc2_app_kit::NSWindow;
            let overlay = handle
                .get_webview_window(mod_screenshot::OVERLAY_LABEL)
                .ok_or_else(|| anyhow::anyhow!("capture overlay missing"))?;
            // SAFETY: Tauri 持有存活窗口；仅在 AppKit 主线程借用原生指针。
            let window: &NSWindow = unsafe { &*overlay.ns_window()?.cast::<NSWindow>() };
            Ok(window.level())
        })();
        let _ = sender.send(result);
    })?;
    receiver.recv_timeout(std::time::Duration::from_secs(5))?
}

/// 截图闭环：真实冻结帧 → 隐藏遮罩 → 绘制后显示并提升层级 → 贴图 → 关闭 → 停用清理。
/// 缺屏幕录制权限时验证如实报错且不创建遮罩。
fn check_screenshot_lifecycle(app: &AppHandle, host: &Arc<DesktopHost>) -> anyhow::Result<()> {
    use serde_json::json;
    let overlay = || app.get_webview_window(mod_screenshot::OVERLAY_LABEL);
    if !host.screenshot().permission_status().granted {
        let denied = host.invoke_ipc("screenshot_start_capture", json!({}));
        anyhow::ensure!(
            !denied.ok,
            "capture must fail without screen recording permission"
        );
        anyhow::ensure!(overlay().is_none(), "no overlay without permission");
        println!(
            "UI_SMOKE_NOTE: screenshot capture skipped, permission denied: {:?}",
            denied.error
        );
        return Ok(());
    }
    let started = host.invoke_ipc("screenshot_start_capture", json!({"hide_dashboard": true}));
    anyhow::ensure!(started.ok, "start capture: {:?}", started.error);
    let info = started.data.unwrap();
    let session_id = info["session_id"].as_str().unwrap().to_string();
    let window = overlay().ok_or_else(|| anyhow::anyhow!("capture overlay WebView"))?;
    anyhow::ensure!(
        !window.is_visible()?,
        "overlay must stay hidden before frame is drawn"
    );
    let frame = host.screenshot_frame_bytes(&session_id)?;
    anyhow::ensure!(
        frame.len() as u64
            == info["frame_width"].as_u64().unwrap() * info["frame_height"].as_u64().unwrap() * 4,
        "frozen frame byte length"
    );
    let ready = host.invoke_ipc(
        "screenshot_overlay_ready",
        json!({"session_id": session_id}),
    );
    anyhow::ensure!(ready.ok, "overlay ready: {:?}", ready.error);
    anyhow::ensure!(
        wait_until(|| window.is_visible().unwrap_or(false)),
        "overlay shown"
    );
    #[cfg(target_os = "macos")]
    {
        let level = overlay_native_level(app)?;
        anyhow::ensure!(
            level > objc2_app_kit::NSMainMenuWindowLevel,
            "overlay level {level} must be above the menu bar"
        );
    }
    let pinned = host.screenshot_export(
        &session_id,
        app_core::SelectionBox {
            x: 0,
            y: 0,
            width: 64,
            height: 48,
        },
        mod_screenshot::export::ExportAction::Pin,
        None,
    )?;
    let pin_label = pinned.pin.as_ref().unwrap().label.clone();
    anyhow::ensure!(
        wait_until(|| overlay().is_none() && app.get_webview_window(&pin_label).is_some()),
        "pin replaces overlay"
    );
    let closed = host.invoke_ipc(
        "screenshot_close_pin",
        json!({"pin_id": pinned.pin.unwrap().pin_id}),
    );
    anyhow::ensure!(closed.ok, "close pin: {:?}", closed.error);
    anyhow::ensure!(
        wait_until(|| app.get_webview_window(&pin_label).is_none()),
        "pin destroyed"
    );

    let again = host.invoke_ipc("screenshot_start_capture", json!({"hide_dashboard": true}));
    anyhow::ensure!(again.ok, "second capture: {:?}", again.error);
    host.kernel()
        .set_module_enabled(mod_screenshot::MODULE_ID, false)?;
    anyhow::ensure!(
        wait_until(|| overlay().is_none()),
        "disable destroys capture overlay"
    );
    anyhow::ensure!(
        !host.screenshot().has_active_session(),
        "disable releases frozen frame"
    );
    host.kernel()
        .set_module_enabled(mod_screenshot::MODULE_ID, true)?;
    Ok(())
}

pub(super) fn run_ui_smoke_checks(app: AppHandle, keep_open: bool) {
    std::thread::spawn(move || {
        let host = app.state::<Arc<DesktopHost>>().inner().clone();
        let data_dir = host.kernel().context().app_data_dir.clone();
        let result = (|| -> anyhow::Result<()> {
            let dashboard = host.invoke_ipc("kernel_open_dashboard", serde_json::json!({}));
            anyhow::ensure!(
                dashboard.ok && app.get_webview_window("dashboard").is_some(),
                "dashboard WebView"
            );
            let modules = host.module_catalog();
            anyhow::ensure!(
                modules.as_array().unwrap().len() == 10,
                "ten building blocks"
            );
            anyhow::ensure!(
                modules
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|m| m["available"] == true)
                    .count()
                    == 9,
                "nine mounted feature modules"
            );
            check_screenshot_lifecycle(&app, &host)?;
            host.kernel().trigger_action(ACTION_CLIPBOARD)?;
            anyhow::ensure!(
                app.get_webview_window("clipboard-popup").is_some(),
                "clipboard WebView"
            );
            host.kernel().set_module_enabled("clipboard", false)?;
            anyhow::ensure!(
                wait_until(|| app.get_webview_window("clipboard-popup").is_none()),
                "disable closes clipboard WebView"
            );
            anyhow::ensure!(
                host.kernel().trigger_action(ACTION_CLIPBOARD).is_err(),
                "disabled shortcut"
            );
            host.kernel().set_module_enabled("clipboard", true)?;
            register_global_shortcuts(&app)?;
            let note = host.invoke_ipc(
                "sticky_create_note",
                serde_json::json!({"title": "Module lifecycle smoke", "content_md": "**saved**"}),
            );
            anyhow::ensure!(note.ok, "create note: {:?}", note.error);
            let id = note.data.unwrap()["id"].as_str().unwrap().to_string();
            let label = format!("sticky-note-{id}");
            anyhow::ensure!(
                app.get_webview_window(&label).is_some(),
                "sticky note WebView"
            );
            host.kernel().set_module_enabled("sticky_note", false)?;
            anyhow::ensure!(
                wait_until(|| app.get_webview_window(&label).is_none()),
                "disable closes sticky WebView"
            );
            host.kernel().set_module_enabled("sticky_note", true)?;
            anyhow::ensure!(
                app.get_webview_window(&label).is_some(),
                "enable restores sticky WebView"
            );
            let _ = host.invoke_ipc("sticky_close_note", serde_json::json!({"id": id}));

            // 新增注释模块必须创建两个真实窗口，并在停用后同时释放。
            let ink = host.invoke_ipc("ink_start_session", serde_json::json!({}));
            anyhow::ensure!(ink.ok, "start screen ink: {:?}", ink.error);
            anyhow::ensure!(
                app.get_webview_window("screen-ink-canvas").is_some()
                    && app.get_webview_window("screen-ink-toolbar").is_some(),
                "screen ink canvas and independent toolbar"
            );
            #[cfg(target_os = "macos")]
            {
                // 模拟画布绘制抢回焦点，再检查工具栏仍在画布前方，而非仅检查启动顺序。
                app.get_webview_window("screen-ink-canvas")
                    .unwrap()
                    .set_focus()?;
                check_ink_toolbar_stacking(&app)
                    .map_err(|error| anyhow::anyhow!("after canvas focus: {error}"))?;
            }
            let passthrough = host.invoke_ipc(
                "ink_set_passthrough",
                serde_json::json!({"passthrough": true}),
            );
            anyhow::ensure!(
                passthrough.ok,
                "screen ink passthrough: {:?}",
                passthrough.error
            );
            // 截图恢复会依次 show 两个窗口，恢复后也必须维持工具栏层级。
            for label in ["screen-ink-canvas", "screen-ink-toolbar"] {
                host.kernel()
                    .context()
                    .window_pool
                    .set_window_visible(label, false)?;
            }
            for label in ["screen-ink-canvas", "screen-ink-toolbar"] {
                host.kernel()
                    .context()
                    .window_pool
                    .set_window_visible(label, true)?;
            }
            #[cfg(target_os = "macos")]
            check_ink_toolbar_stacking(&app)
                .map_err(|error| anyhow::anyhow!("after hide/show: {error}"))?;
            host.kernel().set_module_enabled("screen_ink", false)?;
            anyhow::ensure!(
                wait_until(|| app.get_webview_window("screen-ink-canvas").is_none()
                    && app.get_webview_window("screen-ink-toolbar").is_none()),
                "disable destroys both screen ink WebViews"
            );
            host.kernel().set_module_enabled("screen_ink", true)?;

            // 保存脚本只验证库与接入，不执行脚本或读取系统剪贴板。
            let script = host.invoke_ipc(
                "script_save",
                serde_json::json!({
                    "name": "Native integration smoke", "script_type": "shell",
                    "content_or_path": "echo should-not-execute-on-save"
                }),
            );
            anyhow::ensure!(script.ok, "script library IPC: {:?}", script.error);
            let runs = host.invoke_ipc("script_list_runs", serde_json::json!({}));
            anyhow::ensure!(
                runs.ok && runs.data.unwrap().as_array().unwrap().is_empty(),
                "save does not execute scripts"
            );
            host.kernel().set_module_enabled("script_runner", false)?;
            anyhow::ensure!(
                !host.invoke_ipc("script_list", serde_json::json!({})).ok,
                "disabled script IPC"
            );
            host.kernel().set_module_enabled("script_runner", true)?;
            let media = host.invoke_ipc("media_list_jobs", serde_json::json!({}));
            anyhow::ensure!(media.ok, "media queue IPC: {:?}", media.error);
            host.kernel().set_module_enabled("media_proc", false)?;
            anyhow::ensure!(
                !host.invoke_ipc("media_list_jobs", serde_json::json!({})).ok,
                "disabled media IPC"
            );
            host.kernel().set_module_enabled("media_proc", true)?;
            register_global_shortcuts(&app)?;
            update_tray_menu(&app)?;
            Ok(())
        })();
        match &result {
            Ok(_) => println!("UI_SMOKE_RESULT: PASS (native windows, module switches, cleanup, restore, shortcuts)"),
            Err(error) => eprintln!("UI_SMOKE_RESULT: FAIL: {error:#}"),
        }
        if keep_open && result.is_ok() {
            emit_update(&app);
        } else {
            let _ = host.kernel().shutdown();
            let _ = std::fs::remove_dir_all(data_dir);
            app.exit(if result.is_ok() { 0 } else { 1 });
        }
    });
}
