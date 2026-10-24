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
