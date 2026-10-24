//! 从既有三模块数据库升级，验证新增三个积木能共存且开关互不干扰。
use all_in_one_host::DesktopHost;
use app_core::AppKernelBuilder;
use mod_clipboard::ClipboardModule;
use mod_sticky_note::StickyNoteModule;
use mod_window_pin::WindowPinModule;
use os_hal::MockOsHal;
use serde_json::{json, Value};
use std::sync::Arc;

fn call(host: &DesktopHost, cmd: &str, args: Value) -> Value {
    let response = host.invoke_ipc(cmd, args);
    assert!(response.ok, "{cmd}: {:?}", response.error);
    response.data.unwrap()
}

#[test]
fn upgrade_preserves_old_data_and_new_feature_switches_are_independent() {
    let dir = tempfile::tempdir().unwrap();
    let hal = Arc::new(MockOsHal::new());
    let clipboard = Arc::new(ClipboardModule::new(hal.clone()));
    let sticky = Arc::new(StickyNoteModule::new());
    let old = AppKernelBuilder::new(dir.path())
        .register_module_arc(clipboard.clone())
        .unwrap()
        .register_module_arc(sticky.clone())
        .unwrap()
        .register_module(WindowPinModule::new(hal))
        .unwrap()
        .bootstrap()
        .unwrap();
    clipboard
        .capture_text(old.context(), "pre-upgrade clipboard", None)
        .unwrap();
    let note = sticky
        .create_note(
            old.context(),
            Some("before upgrade"),
            Some("**keep**"),
            None,
        )
        .unwrap();
    old.shutdown().unwrap();
    drop(old);

    let host = DesktopHost::bootstrap(dir.path(), false).unwrap();
    let catalog = call(&host, "kernel_list_modules", json!({}));
    let ids: Vec<_> = catalog
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["available"] == true)
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            "clipboard",
            "screen_ink",
            "sticky_note",
            "script_runner",
            "translator",
            "media_proc",
            "window_pin"
        ]
    );
    assert_eq!(
        call(&host, "clipboard_list_items", json!({}))[0]["content_text"],
        "pre-upgrade clipboard"
    );
    assert_eq!(
        call(&host, "sticky_list_notes", json!({}))[0]["id"],
        note.id
    );
    let script = call(
        &host,
        "script_save",
        json!({"name":"keep script", "script_type":"shell", "content_or_path":"echo should-not-run-on-save"}),
    );
    assert!(call(&host, "script_list_runs", json!({}))
        .as_array()
        .unwrap()
        .is_empty());
    call(
        &host,
        "media_update_config",
        json!({"max_concurrent_jobs": 1}),
    );
    call(&host, "ink_start_session", json!({}));
    call(
        &host,
        "kernel_set_module_enabled",
        json!({"module_id":"screen_ink", "enabled":false}),
    );
    assert!(!host.invoke_ipc("ink_get_status", json!({})).ok);
    assert!(host.kernel().registry().is_enabled("script_runner"));
    assert_eq!(call(&host, "script_list", json!({}))[0]["id"], script["id"]);
    assert_eq!(
        call(&host, "media_get_config", json!({}))["max_concurrent_jobs"],
        1
    );
    // 原有便笺继续驻留；只销毁被停用的注释双窗口。
    assert_eq!(host.kernel().context().window_pool.active_window_count(), 1);
    call(
        &host,
        "kernel_set_module_enabled",
        json!({"module_id":"script_runner", "enabled":false}),
    );
    assert!(call(&host, "media_list_jobs", json!({}))
        .as_array()
        .unwrap()
        .is_empty());
    host.kernel().shutdown().unwrap();
    drop(host);

    let rebooted = DesktopHost::bootstrap(dir.path(), false).unwrap();
    assert!(!rebooted.kernel().registry().is_enabled("screen_ink"));
    assert!(!rebooted.kernel().registry().is_enabled("script_runner"));
    assert!(rebooted.kernel().registry().is_enabled("media_proc"));
    call(
        &rebooted,
        "kernel_set_module_enabled",
        json!({"module_id":"script_runner", "enabled":true}),
    );
    assert_eq!(
        call(&rebooted, "script_list", json!({}))[0]["id"],
        script["id"]
    );
    assert_eq!(
        call(&rebooted, "media_get_config", json!({}))["max_concurrent_jobs"],
        1
    );
    rebooted.kernel().shutdown().unwrap();
    assert!(dir
        .path()
        .join("backups")
        .read_dir()
        .unwrap()
        .next()
        .is_some());
}
