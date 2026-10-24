//! 主程序功能积木集成测试，显式使用 Mock 系统能力。
use all_in_one_host::DesktopHost;
use serde_json::{json, Value};

fn call(host: &DesktopHost, cmd: &str, args: Value) -> Value {
    let response = host.invoke_ipc(cmd, args);
    assert!(response.ok, "{cmd}: {:?}", response.error);
    response.data.unwrap()
}
fn toggle(host: &DesktopHost, id: &str, enabled: bool) {
    call(
        host,
        "kernel_set_module_enabled",
        json!({"module_id": id, "enabled": enabled}),
    );
}

#[test]
fn catalog_has_ten_slots_and_unimplemented_blocks_cannot_be_enabled() {
    let dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
    let modules = call(&host, "kernel_list_modules", json!({}));
    let modules = modules.as_array().unwrap();
    assert_eq!(modules.len(), 10);
    assert_eq!(modules.iter().filter(|m| m["available"] == true).count(), 4);
    assert!(
        !host
            .invoke_ipc(
                "kernel_set_module_enabled",
                json!({"module_id": "screenshot", "enabled": true})
            )
            .ok
    );
    assert!(
        !host
            .invoke_ipc(
                "kernel_set_module_enabled",
                json!({"module_id": "unknown", "enabled": true})
            )
            .ok
    );
    assert_eq!(host.kernel().context().window_pool.active_window_count(), 0);
}

#[test]
fn clipboard_disable_blocks_ipc_and_shortcuts_without_losing_data() {
    let dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
    let item = call(
        &host,
        "clipboard_capture_text",
        json!({"text": "keep this record"}),
    );
    call(
        &host,
        "kernel_trigger_shortcut",
        json!({"action_id": "clipboard:toggle_popup"}),
    );
    toggle(&host, "clipboard", false);
    assert_eq!(host.kernel().context().window_pool.active_window_count(), 0);
    assert!(!host.invoke_ipc("clipboard_list_items", json!({})).ok);
    assert!(
        !host
            .invoke_ipc(
                "clipboard_capture_text",
                json!({"text": "must not capture"})
            )
            .ok
    );
    assert!(host
        .kernel()
        .trigger_action("clipboard:toggle_popup")
        .is_err());
    // Other blocks continue to work.
    call(&host, "sticky_list_notes", json!({}));
    toggle(&host, "clipboard", true);
    let items = call(&host, "clipboard_list_items", json!({}));
    assert_eq!(items.as_array().unwrap().len(), 1);
    assert_eq!(items[0]["id"], item["id"]);
    assert!(host
        .kernel()
        .context()
        .shortcut_registry
        .get_by_action("clipboard:toggle_popup")
        .is_some());
}

#[test]
fn sticky_switch_is_durable_and_restores_windows_only_when_enabled() {
    let dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap(dir.path(), false).unwrap();
    let note = call(
        &host,
        "sticky_create_note",
        json!({"title": "keep", "content_md": "**saved**"}),
    );
    toggle(&host, "sticky_note", false);
    assert_eq!(host.kernel().context().window_pool.active_window_count(), 0);
    host.kernel().shutdown().unwrap();
    drop(host);
    let host = DesktopHost::bootstrap(dir.path(), false).unwrap();
    assert!(!host.kernel().registry().is_enabled("sticky_note"));
    assert_eq!(host.kernel().context().window_pool.active_window_count(), 0);
    assert!(!host.invoke_ipc("sticky_create_note", json!({})).ok);
    toggle(&host, "sticky_note", true);
    assert_eq!(host.kernel().context().window_pool.active_window_count(), 1);
    let notes = call(&host, "sticky_list_notes", json!({}));
    assert_eq!(notes[0]["id"], note["id"]);
    assert_eq!(notes[0]["content_md"], "**saved**");
    host.kernel().shutdown().unwrap();
    host.kernel().shutdown().unwrap();
}

#[test]
fn disabling_window_pin_restores_external_windows_and_closes_picker() {
    let dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
    call(&host, "winpin_toggle_handle", json!({"handle": 1001}));
    call(
        &host,
        "kernel_trigger_shortcut",
        json!({"action_id": "window_pin:pick_cursor"}),
    );
    toggle(&host, "window_pin", false);
    assert_eq!(host.kernel().context().window_pool.active_window_count(), 0);
    assert!(
        !host
            .invoke_ipc("winpin_toggle_handle", json!({"handle": 1001}))
            .ok
    );
    toggle(&host, "window_pin", true);
    let windows = call(&host, "winpin_list_candidates", json!({}));
    assert!(windows["pinned"].as_array().unwrap().is_empty());
    assert!(windows["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .all(|w| w["is_topmost"] == false));
}

#[test]
fn v2_config_migration_preserves_existing_settings_and_unknown_fields() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("config.json"),
        serde_json::to_vec(&json!({
            "config_version": 2, "clipboard": {"max_history_items": 37, "enabled": false},
            "custom_future_setting": {"keep": true},
            "modules": {"enabled": {"clipboard": false}}
        }))
        .unwrap(),
    )
    .unwrap();
    let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
    assert!(!host.kernel().registry().is_enabled("clipboard"));
    let cfg = host.kernel().context().config.read();
    assert_eq!(cfg.config_version, app_core::CURRENT_CONFIG_VERSION);
    assert_eq!(cfg.clipboard.max_history_items, 37);
    assert!(!cfg.clipboard.enabled);
    assert_eq!(cfg.unknown_fields["custom_future_setting"]["keep"], true);
}

#[test]
fn general_config_language_update_persists_across_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap(dir.path(), false).unwrap();

    // 默认语言应为 zh-CN
    let initial_status = call(&host, "kernel_get_status", json!({}));
    assert_eq!(initial_status["config"]["general"]["language"], "zh-CN");

    // 切换语言为 en-US 并验证回包与磁盘持久化
    let updated_cfg = call(
        &host,
        "kernel_update_general_config",
        json!({ "language": "en-US", "theme": "dark" }),
    );
    assert_eq!(updated_cfg["general"]["language"], "en-US");
    assert_eq!(updated_cfg["general"]["theme"], "dark");
    host.kernel().shutdown().unwrap();
    drop(host);

    // 重新启动宿主，确认语言与主题配置从 config.json 恢复
    let rebooted = DesktopHost::bootstrap(dir.path(), false).unwrap();
    let reboot_status = call(&rebooted, "kernel_get_status", json!({}));
    assert_eq!(reboot_status["config"]["general"]["language"], "en-US");
    assert_eq!(reboot_status["config"]["general"]["theme"], "dark");
    rebooted.kernel().shutdown().unwrap();
}

#[test]
fn script_ipc_crud_run_and_disable_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap(dir.path(), true).unwrap();

    let probe = call(&host, "script_probe_interpreters", json!({}));
    let shell_ready = probe["interpreters"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["script_type"] == "shell" && item["available"] == true);
    assert!(probe["environment"].is_object());

    let saved = call(
        &host,
        "script_save",
        json!({"script": {
            "name": "ipc script",
            "script_type": "shell",
            "is_inline": true,
            "content_or_path": "printf 'ipc-out\\n'; printf 'ipc-err\\n' >&2; exit 2\n",
            "args": [],
            "timeout_secs": 15,
            "output_handling": "stream"
        }}),
    );
    let script_id = saved["id"].as_i64().unwrap();
    assert_eq!(saved["name"], "ipc script");
    assert_eq!(
        call(&host, "script_list", json!({}))
            .as_array()
            .unwrap()
            .len(),
        1
    );

    if shell_ready {
        let run = call(
            &host,
            "script_execute",
            json!({"script_id": script_id, "args": []}),
        );
        let run_id = run["run_id"].as_str().unwrap().to_string();
        let record = loop {
            let runs = call(&host, "script_list_runs", json!({"script_id": script_id}));
            let record = runs.as_array().unwrap().first().cloned().unwrap();
            if record["status"] != "running" {
                break record;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        };
        assert_eq!(record["run_id"], run_id);
        assert_eq!(record["status"], "failed");
        assert_eq!(record["exit_code"], 2);
        assert!(record["stdout_tail"].as_str().unwrap().contains("ipc-out"));
        assert!(record["stderr_tail"].as_str().unwrap().contains("ipc-err"));
        let logs = call(&host, "script_get_logs", json!({"run_id": run_id}));
        assert!(!logs["live"].as_bool().unwrap());
        assert!(logs["run"].is_object());
    }

    // 长时间运行的脚本在关闭模块时被强制终止，执行记录保留。
    if shell_ready {
        let sleeping = call(
            &host,
            "script_save",
            json!({"script": {
                "name": "sleep script",
                "script_type": "shell",
                "is_inline": true,
                "content_or_path": "sleep 30\n",
                "timeout_secs": 60,
                "output_handling": "stream"
            }}),
        );
        let sleeping_id = sleeping["id"].as_i64().unwrap();
        let running = call(
            &host,
            "script_execute",
            json!({"script_id": sleeping_id, "args": []}),
        );
        let running_run_id = running["run_id"].as_str().unwrap().to_string();
        std::thread::sleep(std::time::Duration::from_millis(120));
        toggle(&host, "script_runner", false);
        assert!(!host.invoke_ipc("script_list", json!({})).ok);
        assert!(
            !host
                .invoke_ipc("script_execute", json!({"script_id": sleeping_id}))
                .ok
        );
        toggle(&host, "script_runner", true);
        let runs = call(&host, "script_list_runs", json!({"limit": 10}));
        let stopped = runs
            .as_array()
            .unwrap()
            .iter()
            .find(|run| run["run_id"] == running_run_id.as_str())
            .cloned()
            .expect("run record survives module disable");
        assert_eq!(stopped["status"], "cancelled");
    }

    // 删除脚本不删除历史执行记录。
    call(&host, "script_delete", json!({"script_id": script_id}));
    let scripts = call(&host, "script_list", json!({}));
    assert!(!scripts
        .as_array()
        .unwrap()
        .iter()
        .any(|script| script["id"] == script_id));
}

#[test]
fn script_settings_validate_and_persist_across_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap(dir.path(), false).unwrap();
    assert!(!host
        .invoke_ipc(
            "script_update_settings",
            json!({"settings": {"max_log_bytes": 1, "default_timeout_secs": 0, "keep_run_logs": 1}})
        )
        .ok);
    let config = call(
        &host,
        "script_update_settings",
        json!({"settings": {"max_log_bytes": 8192, "default_timeout_secs": 12, "keep_run_logs": 25}}),
    );
    assert_eq!(config["max_log_bytes"], 8192);
    assert_eq!(config["default_timeout_secs"], 12);
    assert_eq!(config["keep_run_logs"], 25);
    host.kernel().shutdown().unwrap();
    drop(host);

    let host = DesktopHost::bootstrap(dir.path(), false).unwrap();
    let config = call(&host, "script_get_settings", json!({}));
    assert_eq!(config["max_log_bytes"], 8192);
    assert_eq!(config["default_timeout_secs"], 12);
    assert_eq!(config["keep_run_logs"], 25);
    host.kernel().shutdown().unwrap();
}
