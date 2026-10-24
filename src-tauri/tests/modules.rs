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
fn media_ipc_runs_real_image_jobs_persists_settings_and_honors_disable() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(".runtime_data")
        .join("host-media-tests");
    std::fs::create_dir_all(&root).unwrap();
    let dir = tempfile::Builder::new()
        .prefix("ipc-")
        .tempdir_in(root)
        .unwrap();
    let input = dir.path().join("IPC 图片.png");
    image::RgbImage::new(10, 6).save(&input).unwrap();
    let host = DesktopHost::bootstrap(dir.path(), false).unwrap();
    let environment = call(&host, "media_check_ffmpeg_env", json!({}));
    assert_eq!(environment["image_formats"].as_array().unwrap().len(), 5);
    let output_dir = dir.path().join("输出 目录");
    call(
        &host,
        "media_update_config",
        json!({"output_dir": output_dir, "max_concurrent_jobs": 1}),
    );
    let jobs = call(
        &host,
        "media_convert_images",
        json!({
            "input_paths": [input],
            "output_dir": null,
            "options": {
                "output_format": "jpeg", "quality": 83, "width": 5,
                "height": null, "preserve_aspect": true
            }
        }),
    );
    let job_id = jobs[0]["id"].as_str().unwrap().to_string();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let records = call(&host, "media_list_jobs", json!({}));
        let job = records
            .as_array()
            .unwrap()
            .iter()
            .find(|job| job["id"] == job_id)
            .unwrap();
        if job["status"] == "completed" {
            assert_eq!(job["progress"], 100.0);
            assert!(job["output_path"].as_str().unwrap().contains("输出 目录"));
            break;
        }
        assert_ne!(job["status"], "failed", "{job}");
        assert!(std::time::Instant::now() < deadline, "media job timed out");
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    toggle(&host, "media_proc", false);
    assert!(!host.invoke_ipc("media_list_jobs", json!({})).ok);
    assert!(
        !host
            .invoke_ipc("media_inspect_file", json!({"input_path": "x"}))
            .ok
    );
    host.kernel().shutdown().unwrap();
    drop(host);

    let host = DesktopHost::bootstrap(dir.path(), false).unwrap();
    assert!(!host.kernel().registry().is_enabled("media_proc"));
    let config: mod_media_proc::MediaProcConfig = host
        .kernel()
        .context()
        .module_settings("media_proc")
        .unwrap();
    assert_eq!(config.max_concurrent_jobs, 1);
    assert_eq!(config.output_dir.as_deref(), Some(output_dir.as_path()));
    toggle(&host, "media_proc", true);
    let records = call(&host, "media_list_jobs", json!({}));
    assert_eq!(records.as_array().unwrap().len(), 1);
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
