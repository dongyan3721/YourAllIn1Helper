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
