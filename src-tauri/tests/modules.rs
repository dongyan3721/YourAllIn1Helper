//! 主程序功能积木集成测试，显式使用 Mock 系统能力。
use all_in_one_host::DesktopHost;
use app_core::{HeadlessWindowBackend, RegisteredShortcut};
use mod_script_runner::events::NoopScriptEventSink;
use mod_script_runner::hotkey::ScriptHotkeySync;
use mod_script_runner::UnavailableScriptEnvironment;
use os_hal::MockOsHal;
use serde_json::{json, Value};
use std::sync::Arc;

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
    assert_eq!(modules.iter().filter(|m| m["available"] == true).count(), 9);
    assert!(
        !host
            .invoke_ipc(
                "kernel_set_module_enabled",
                json!({"module_id": "translator", "enabled": true})
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
fn media_native_pickers_require_async_desktop_adapter_and_enabled_module() {
    let dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
    host.kernel()
        .set_module_enabled("media_proc", false)
        .unwrap();
    for command in ["media_pick_files", "media_pick_directory"] {
        let disabled = host.invoke_ipc(command, serde_json::json!({}));
        assert!(!disabled.ok, "disabled picker unexpectedly succeeded");
        assert!(disabled.error.unwrap().contains("disabled"));

        host.kernel()
            .set_module_enabled("media_proc", true)
            .unwrap();
        let sync = host.invoke_ipc(command, serde_json::json!({}));
        assert!(
            !sync.ok,
            "synchronous picker entry must not launch a process"
        );
        assert!(sync
            .error
            .unwrap()
            .contains("asynchronous desktop IPC adapter"));
        host.kernel()
            .set_module_enabled("media_proc", false)
            .unwrap();
    }
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

#[test]
fn screen_ink_disable_blocks_ipc_and_cleans_windows() {
    let dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap(dir.path(), true).unwrap();

    // 1. 获取初始状态
    let status = call(&host, "ink_get_status", json!({}));
    assert_eq!(status["is_active"], false);

    // 2. 启动会话 -> 创建 canvas 和 toolbar 双窗口
    let start_res = call(&host, "ink_start_session", json!({}));
    assert_eq!(start_res["is_active"], true);
    assert_eq!(host.kernel().context().window_pool.active_window_count(), 2);

    // 3. 切换为穿透模式
    let pass_res = call(&host, "ink_set_passthrough", json!({ "passthrough": true }));
    assert_eq!(pass_res["is_passthrough"], true);
    let canvas_win = host
        .kernel()
        .context()
        .window_pool
        .get_active_window("screen-ink-canvas")
        .unwrap();
    assert!(canvas_win.ignore_cursor_events);
    let toolbar_win = host
        .kernel()
        .context()
        .window_pool
        .get_active_window("screen-ink-toolbar")
        .unwrap();
    assert!(!toolbar_win.ignore_cursor_events);

    // 4. 保存快照合成（生成一个微型 2x2 绿色透明 PNG 并 base64 编码测试）
    use image::{ImageBuffer, Rgba};
    use std::io::Cursor;
    let mut img = ImageBuffer::<Rgba<u8>, Vec<u8>>::new(2, 2);
    img.put_pixel(0, 0, Rgba([0, 255, 0, 255]));
    let mut png_buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut png_buf), image::ImageFormat::Png)
        .unwrap();
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let b64 = format!("data:image/png;base64,{}", STANDARD.encode(&png_buf));

    let snap_res = call(
        &host,
        "ink_save_snapshot",
        json!({
            "ink_png_base64": b64,
            "copy_to_clipboard": false,
        }),
    );
    assert!(snap_res["width"].as_u64().unwrap() > 0);
    assert!(snap_res["height"].as_u64().unwrap() > 0);

    // 5. 禁用 screen_ink 模块 -> 双窗口被销毁，内存释放
    toggle(&host, "screen_ink", false);
    assert_eq!(host.kernel().context().window_pool.active_window_count(), 0);

    // 6. 禁用状态下所有 ink_ IPC 命令必须被拒绝
    assert!(!host.invoke_ipc("ink_get_status", json!({})).ok);
    assert!(!host.invoke_ipc("ink_start_session", json!({})).ok);
    assert!(
        !host
            .invoke_ipc("ink_set_passthrough", json!({ "passthrough": false }))
            .ok
    );

    // 7. 重新启用
    toggle(&host, "screen_ink", true);
    let re_status = call(&host, "ink_get_status", json!({}));
    assert_eq!(re_status["is_active"], false);

    host.kernel().shutdown().unwrap();
}

#[test]
fn script_save_uses_applescript_and_powershell_json_contract_with_legacy_aliases() {
    let dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
    let apple = call(
        &host,
        "script_save",
        json!({"script": {
            "name": "apple",
            "script_type": "apple_script",
            "is_inline": true,
            "content_or_path": "return 1\n",
            "timeout_secs": 10
        }}),
    );
    assert_eq!(apple["script_type"], "applescript");

    let powershell = call(
        &host,
        "script_save",
        json!({"script": {
            "name": "powershell",
            "script_type": "power_shell",
            "is_inline": true,
            "content_or_path": "Write-Output 1\n",
            "timeout_secs": 10
        }}),
    );
    assert_eq!(powershell["script_type"], "powershell");

    let saved = call(
        &host,
        "script_save",
        json!({"script": {
            "name": "apple-new",
            "script_type": "applescript",
            "is_inline": true,
            "content_or_path": "return 2\n",
            "timeout_secs": 10
        }}),
    );
    assert_eq!(saved["script_type"], "applescript");

    let list = call(&host, "script_list", json!({}));
    let values: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["script_type"].as_str().unwrap())
        .collect();
    assert!(values.contains(&"applescript"));
    assert!(values.contains(&"powershell"));
    assert!(!values.iter().any(|value| value.contains('_')));
}

struct FailingNativeHotkeys;

impl ScriptHotkeySync for FailingNativeHotkeys {
    fn sync(
        &self,
        _action_id: &str,
        _previous: Option<&RegisteredShortcut>,
        _desired: Option<&RegisteredShortcut>,
    ) -> Result<(), String> {
        Err("injected native hotkey registration failure".to_string())
    }
}

#[test]
fn host_script_save_reports_and_rolls_back_native_hotkey_failure() {
    let dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap_with_adapters_and_services(
        dir.path(),
        true,
        Arc::new(HeadlessWindowBackend),
        MockOsHal::new().into_bundle(),
        Arc::new(UnavailableScriptEnvironment),
        Arc::new(NoopScriptEventSink),
        Arc::new(FailingNativeHotkeys),
    )
    .unwrap();

    let response = host.invoke_ipc(
        "script_save",
        json!({"script": {
            "name": "must roll back",
            "script_type": "shell",
            "is_inline": true,
            "content_or_path": "echo hi
        ",
            "hotkey": "PrimaryMod+Alt+F4",
            "timeout_secs": 10
        }}),
    );
    assert!(!response.ok);
    let error = response.error.unwrap();
    assert!(error.contains("系统全局快捷键更新失败"), "{error}");
    let scripts = call(&host, "script_list", json!({}));
    assert!(scripts.as_array().unwrap().is_empty());
}

#[test]
fn app_usage_ipc_reports_capabilities_persists_config_and_blocks_when_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap(dir.path(), true).unwrap();

    let status = call(&host, "usage_get_status", json!({}));
    assert_eq!(status["tracking"], true);
    assert_eq!(status["capabilities"]["idle_detection"], true);
    assert_eq!(status["capabilities"]["screen_lock_detection"], true);
    assert_eq!(status["capabilities"]["audio_afk_exception"], false);
    assert!(status["capabilities"]["limitations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|line| line.as_str().unwrap_or_default().contains("音频")));

    let config = call(
        &host,
        "usage_update_config",
        json!({"idle_threshold_secs": 120, "flush_interval_secs": 15, "keep_window_titles": false}),
    );
    assert_eq!(config["idle_threshold_secs"], 120);
    assert_eq!(config["flush_interval_secs"], 15);
    assert_eq!(config["keep_window_titles"], false);
    assert_eq!(
        host.kernel()
            .context()
            .config
            .read()
            .modules
            .settings
            .get("app_usage")
            .unwrap()["idle_threshold_secs"],
        120
    );

    let overview = call(
        &host,
        "usage_get_overview",
        json!({"start_date": "2026-10-01", "end_date": "2026-10-25"}),
    );
    assert_eq!(overview["days"].as_array().unwrap().len(), 25);
    assert_eq!(overview["total_seconds"], 0);
    assert!(call(&host, "usage_get_apps", json!({}))
        .as_array()
        .unwrap()
        .is_empty());

    assert!(
        !host
            .invoke_ipc(
                "usage_set_app_category",
                json!({"app_id": "never.tracked", "category": "work"})
            )
            .ok
    );

    toggle(&host, "app_usage", false);
    assert!(!host.invoke_ipc("usage_get_overview", json!({})).ok);
    toggle(&host, "app_usage", true);
    assert_eq!(call(&host, "usage_get_status", json!({}))["tracking"], true);
    call(&host, "kernel_shutdown", json!({}));
    drop(host);

    // 配置写入 config.json，重启后仍保留；历史数据库由本测试之外的迁移测试覆盖。
    let restarted = DesktopHost::bootstrap(dir.path(), true).unwrap();
    let restored = call(&restarted, "usage_get_config", json!({}));
    assert_eq!(restored["idle_threshold_secs"], 120);
    assert_eq!(restored["flush_interval_secs"], 15);
    assert_eq!(restored["keep_window_titles"], false);
    call(&restarted, "kernel_shutdown", json!({}));
}
