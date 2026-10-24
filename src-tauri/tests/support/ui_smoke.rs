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
                    == 6,
                "six mounted feature modules"
            );
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
            let passthrough = host.invoke_ipc(
                "ink_set_passthrough",
                serde_json::json!({"passthrough": true}),
            );
            anyhow::ensure!(
                passthrough.ok,
                "screen ink passthrough: {:?}",
                passthrough.error
            );
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
