//! 增强截图宿主接线集成测试：显式使用 Mock 系统能力与无桌面窗口后端，
//! 不读取真实屏幕、不写系统剪贴板，文件只写入临时目录。
use all_in_one_host::DesktopHost;
use app_core::SelectionBox;
use mod_screenshot::export::ExportAction;
use serde_json::{json, Value};

fn call(host: &DesktopHost, cmd: &str, args: Value) -> Value {
    let response = host.invoke_ipc(cmd, args);
    assert!(response.ok, "{cmd}: {:?}", response.error);
    response.data.unwrap()
}

#[test]
fn capture_save_pin_flow_through_json_and_binary_entry_points() {
    let dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
    // 启动不抓屏、不建窗口。
    assert_eq!(host.kernel().context().window_pool.active_window_count(), 0);
    let status = call(&host, "screenshot_get_status", json!({}));
    assert!(status["session"].is_null());
    assert_eq!(status["shortcut"]["registered"], true);
    assert!(status["native_shortcut_error"].is_null());

    let info = call(&host, "screenshot_start_capture", json!({}));
    let session_id = info["session_id"].as_str().unwrap().to_string();
    let (w, h) = (
        info["frame_width"].as_u64().unwrap() as u32,
        info["frame_height"].as_u64().unwrap() as u32,
    );
    assert_eq!(
        host.screenshot_frame_bytes(&session_id).unwrap().len(),
        (w * h * 4) as usize
    );
    assert!(host
        .kernel()
        .context()
        .window_pool
        .get_active_window("capture-overlay")
        .is_some());
    call(
        &host,
        "screenshot_overlay_ready",
        json!({"session_id": session_id}),
    );
    // 帧外坐标没有智能区域，帧内至少命中显示器根节点。
    assert!(call(
        &host,
        "screenshot_resolve_region",
        json!({"session_id": session_id, "x": -5, "y": 3})
    )
    .is_null());
    assert_eq!(
        call(
            &host,
            "screenshot_resolve_region",
            json!({"session_id": session_id, "x": 1, "y": 1, "level_steps": 9})
        )["level"],
        "monitor"
    );

    // 宿主未注入系统剪贴板：复制如实失败，会话保留。
    let copy = host.screenshot_export(
        &session_id,
        SelectionBox {
            x: 0,
            y: 0,
            width: 8,
            height: 8,
        },
        ExportAction::Copy,
        None,
    );
    assert!(copy.unwrap_err().to_string().contains("未接入系统剪贴板"));

    let saved = host
        .screenshot_export(
            &session_id,
            SelectionBox {
                x: 0,
                y: 0,
                width: 8,
                height: 8,
            },
            ExportAction::Save,
            None,
        )
        .unwrap();
    let path = std::path::PathBuf::from(saved.saved_path.unwrap());
    assert!(path.starts_with(dir.path().join("screenshots")));
    assert_eq!(image::open(&path).unwrap().width(), 8);
    assert!(host.screenshot_frame_bytes(&session_id).is_err());
    assert_eq!(host.kernel().context().window_pool.active_window_count(), 0);

    // 贴图：PNG 经二进制入口读取，关闭后释放。
    let info = call(&host, "screenshot_start_capture", json!({}));
    let session_id = info["session_id"].as_str().unwrap().to_string();
    let pinned = host
        .screenshot_export(
            &session_id,
            SelectionBox {
                x: 2,
                y: 2,
                width: 6,
                height: 4,
            },
            ExportAction::Pin,
            None,
        )
        .unwrap();
    let pin_id = pinned.pin.unwrap().pin_id;
    assert!(!host.screenshot_pin_png(&pin_id).unwrap().is_empty());
    let pins = call(&host, "screenshot_list_pins", json!({}));
    assert_eq!(pins.as_array().unwrap().len(), 1);
    assert_eq!(
        call(&host, "screenshot_close_pin", json!({"pin_id": pin_id}))["closed"],
        true
    );
    assert!(host.screenshot_pin_png(&pin_id).is_err());
    assert_eq!(host.kernel().context().window_pool.active_window_count(), 0);
    host.kernel().shutdown().unwrap();
}

#[test]
fn disabling_screenshot_rejects_json_and_binary_ipc_and_releases_everything() {
    let dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
    let info = call(&host, "screenshot_start_capture", json!({}));
    let session_id = info["session_id"].as_str().unwrap().to_string();
    let pinned = host
        .screenshot_export(
            &session_id,
            SelectionBox {
                x: 0,
                y: 0,
                width: 4,
                height: 4,
            },
            ExportAction::Pin,
            None,
        )
        .unwrap();
    let pin_id = pinned.pin.unwrap().pin_id;
    let info = call(&host, "screenshot_start_capture", json!({}));
    let live_session = info["session_id"].as_str().unwrap().to_string();
    assert_eq!(host.kernel().context().window_pool.active_window_count(), 2);

    call(
        &host,
        "kernel_set_module_enabled",
        json!({"module_id": "screenshot", "enabled": false}),
    );
    assert_eq!(host.kernel().context().window_pool.active_window_count(), 0);
    for (cmd, args) in [
        ("screenshot_get_status", json!({})),
        ("screenshot_start_capture", json!({})),
        ("screenshot_cancel", json!({})),
        ("screenshot_list_pins", json!({})),
    ] {
        assert!(!host.invoke_ipc(cmd, args).ok, "{cmd} must be rejected");
    }
    assert!(host.screenshot_frame_bytes(&live_session).is_err());
    assert!(host.screenshot_pin_png(&pin_id).is_err());
    assert!(host
        .screenshot_export(
            &live_session,
            SelectionBox {
                x: 0,
                y: 0,
                width: 1,
                height: 1
            },
            ExportAction::Save,
            None
        )
        .is_err());
    assert!(host
        .kernel()
        .trigger_action(mod_screenshot::ACTION_CAPTURE)
        .is_err());

    // 重新启用后旧会话与旧贴图都不会复活。
    call(
        &host,
        "kernel_set_module_enabled",
        json!({"module_id": "screenshot", "enabled": true}),
    );
    let status = call(&host, "screenshot_get_status", json!({}));
    assert!(status["session"].is_null());
    assert!(status["pins"].as_array().unwrap().is_empty());
    host.kernel().shutdown().unwrap();
}

#[test]
fn settings_update_is_atomic_persistent_and_keeps_unknown_fields() {
    let dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap(dir.path(), false).unwrap();
    host.kernel()
        .update_config(|c| {
            c.modules
                .settings
                .insert("screenshot".into(), json!({"from_future_version": [1, 2]}));
        })
        .unwrap();
    let updated = call(
        &host,
        "screenshot_update_settings",
        json!({
            "settings": {"copy_after_save": true, "default_color": "#00ff00"},
            "overlay": {"show_magnifier": false}
        }),
    );
    assert_eq!(updated["settings"]["copy_after_save"], true);
    assert_eq!(updated["overlay"]["show_magnifier"], false);
    // 类型错误整体拒绝，不写入半份配置。
    assert!(
        !host
            .invoke_ipc(
                "screenshot_update_settings",
                json!({"settings": {"max_pins": "many"}, "overlay": {"show_magnifier": true}})
            )
            .ok
    );
    host.kernel().shutdown().unwrap();
    drop(host);

    let host = DesktopHost::bootstrap(dir.path(), false).unwrap();
    let config = call(&host, "screenshot_get_config", json!({}));
    assert_eq!(config["settings"]["copy_after_save"], true);
    assert_eq!(config["settings"]["default_color"], "#00ff00");
    assert_eq!(config["settings"]["from_future_version"], json!([1, 2]));
    assert_eq!(config["overlay"]["show_magnifier"], false);
    host.kernel().shutdown().unwrap();
}

#[test]
fn ocr_ipc_uses_injected_provider_and_intent_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
    let info = call(&host, "screenshot_start_capture", json!({"intent": "ocr"}));
    assert_eq!(info["intent"], "ocr");
    let session_id = info["session_id"].as_str().unwrap().to_string();
    assert_eq!(
        call(&host, "screenshot_get_session", json!({}))["intent"],
        "ocr"
    );
    // 无桌面测试显式注入 MockOsHal，OCR 结果来自测试适配器。
    let out = call(
        &host,
        "screenshot_ocr",
        json!({"session_id": session_id, "selection": {"x": 0, "y": 0, "width": 8, "height": 8}}),
    );
    assert_eq!(
        (out["width"].as_u64(), out["height"].as_u64()),
        (Some(8), Some(8))
    );
    assert!(host.screenshot().has_active_session());
    // 未注入系统剪贴板：复制文字如实失败。
    assert!(
        !host
            .invoke_ipc("screenshot_copy_text", json!({"text": "abc"}))
            .ok
    );
    assert!(
        !host
            .invoke_ipc("screenshot_ocr", json!({"session_id": session_id}))
            .ok
    );
    call(
        &host,
        "kernel_set_module_enabled",
        json!({"module_id": "screenshot", "enabled": false}),
    );
    assert!(!host
        .invoke_ipc(
            "screenshot_ocr",
            json!({"session_id": session_id, "selection": {"x": 0, "y": 0, "width": 8, "height": 8}})
        )
        .ok);
    host.kernel().shutdown().unwrap();
}
