use super::*;

#[test]
fn test_desktop_host_ipc_full_workflow() {
    let temp_dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap(temp_dir.path(), true).unwrap();
    host.seed_demo_data_if_empty().unwrap();

    // 1. Verify Level 0 startup (0 active WebViews)
    let status = host.invoke_ipc("kernel_get_status", json!({}));
    assert!(status.ok);
    let data = status.data.unwrap();
    assert_eq!(data["active_window_count"], 0);
    assert_eq!(data["modules"].as_array().unwrap().len(), 10);

    // 2. Trigger Clipboard shortcut -> opens `clipboard-popup` (1 active WebView)
    let trig = host.invoke_ipc(
        "kernel_trigger_shortcut",
        json!({ "action_id": "clipboard:toggle_popup" }),
    );
    assert!(trig.ok);
    assert_eq!(trig.data.unwrap()["active_window_count"], 1);

    // 3. Capture new clipboard text and paste it -> automatically destroys `clipboard-popup`
    let cap = host.invoke_ipc(
        "clipboard_capture_text",
        json!({ "text": "Hello Desktop IPC", "source_app_name": "Code" }),
    );
    assert!(cap.ok);
    let item_id = cap.data.unwrap()["id"].as_i64().unwrap();

    let pasted = host.invoke_ipc(
        "clipboard_paste_item",
        json!({ "id": item_id, "as_plain_text": true }),
    );
    assert!(pasted.ok);
    let pasted_data = pasted.data.unwrap();
    assert_eq!(pasted_data["active_window_count"], 0);
    assert_eq!(pasted_data["paste_trigger_count"], 1);

    // 4. Create Sticky Note -> opens `sticky-note-{id}` PinnedResident WebView
    let note_resp = host.invoke_ipc(
        "sticky_create_note",
        json!({
            "title": "IPC 测试便笺",
            "content_md": "公式：$\\alpha + \\beta = \\gamma$",
            "theme_color": "emerald"
        }),
    );
    assert!(note_resp.ok);
    let note_id = note_resp.data.unwrap()["id"].as_str().unwrap().to_string();
    assert_eq!(host.kernel().context().window_pool.active_window_count(), 1);

    // Close sticky note -> back to 0 active WebViews
    let close_resp = host.invoke_ipc("sticky_close_note", json!({ "id": note_id }));
    assert!(close_resp.ok);
    assert_eq!(close_resp.data.unwrap()["active_window_count"], 0);

    // 5. Pin external window via crosshair coordinates (250, 200) and verify shutdown auto-restore
    let pick_resp = host.invoke_ipc("winpin_pick_at_cursor", json!({ "x": 250, "y": 200 }));
    assert!(pick_resp.ok);
    assert_eq!(
        pick_resp.data.unwrap()["pinned"].as_array().unwrap().len(),
        1
    );

    let shut_resp = host.invoke_ipc("kernel_shutdown", json!({}));
    assert!(shut_resp.ok);
    assert_eq!(
        shut_resp.data.unwrap()["pinned_external_windows"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn test_translator_ipc_workflow_and_disabled_rejection() {
    let temp_dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap(temp_dir.path(), true).unwrap();

    // 1. 读取配置
    let cfg_resp = host.invoke_ipc("translator_get_config", json!({}));
    assert!(cfg_resp.ok);
    let cfg = cfg_resp.data.unwrap();
    assert_eq!(cfg["default_provider_id"], "google");
    assert_eq!(cfg["target_lang"], "zh-CN");

    // 2. 列出提供商
    let prov_resp = host.invoke_ipc("translator_list_providers", json!({}));
    assert!(prov_resp.ok);
    let providers = prov_resp.data.unwrap().as_array().unwrap().clone();
    assert!(providers.iter().any(|p| p["id"] == "google"));
    assert!(providers.iter().any(|p| p["id"] == "deepl"));
    assert!(providers.iter().any(|p| p["id"] == "openai"));

    // 3. 设置凭据
    let key_resp = host.invoke_ipc(
        "translator_set_api_key",
        json!({ "provider_id": "google", "api_key": "test-key-abc" }),
    );
    assert!(key_resp.ok);

    // 4. 再次列出提供商，验证 has_api_key 变为 true，且绝不包含明文 api_key 字段
    let prov_resp2 = host.invoke_ipc("translator_list_providers", json!({}));
    assert!(prov_resp2.ok);
    let google_meta = prov_resp2
        .data
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "google")
        .cloned()
        .unwrap();
    assert_eq!(google_meta["has_api_key"], true);
    assert!(google_meta.get("api_key").is_none());

    // 5. 提取选区（MockOsHal 预置文本）
    let sel_resp = host.invoke_ipc("translator_extract_selection", json!({}));
    assert!(sel_resp.ok);
    let sel_data = sel_resp.data.unwrap();
    assert_eq!(sel_data["has_selection"], true);
    assert_eq!(sel_data["text"], "Hello All-in-One");

    // 6. 禁用模块：拦截所有 translator_ IPC 调用
    let dis_resp = host.invoke_ipc(
        "kernel_set_module_enabled",
        json!({ "module_id": "translator", "enabled": false }),
    );
    assert!(dis_resp.ok);

    let rej_resp = host.invoke_ipc("translator_get_config", json!({}));
    assert!(!rej_resp.ok);
    assert!(rej_resp
        .error
        .unwrap()
        .contains("Module 'translator' is disabled"));
}

#[tokio::test]
async fn translator_ipc_translation_supports_current_thread_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
    // 无凭据只产生本地错误事件，不访问外部 API。
    let response = host.invoke_ipc(
        "translator_translate",
        json!({"text": "Owned runtime fixture"}),
    );
    assert!(response.ok, "{:?}", response.error);
    assert!(response.data.unwrap()["request_id"].as_u64().is_some());
    host.translator.cancel_current_translation();
}

#[test]
fn translator_ipc_without_runtime_reports_error_instead_of_panicking() {
    let dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
    let response = host.invoke_ipc(
        "translator_translate",
        json!({"text": "Owned runtime fixture"}),
    );
    assert!(!response.ok);
    assert!(response.error.unwrap().contains("异步运行时"));
}

#[test]
fn test_kernel_open_dashboard_translator_route_and_rejects_kernel_open_window() {
    let temp_dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap(temp_dir.path(), true).unwrap();

    // 验证正确的 kernel_open_dashboard 路由调用契约
    let dash_resp = host.invoke_ipc("kernel_open_dashboard", json!({ "route": "/translator" }));
    assert!(dash_resp.ok);
    let data = dash_resp.data.unwrap();
    assert_eq!(data["active_window_count"], 1);
    let active_windows = data["active_windows"].as_array().unwrap();
    let dashboard_win = active_windows
        .iter()
        .find(|w| w["spec"]["label"] == "dashboard")
        .expect("dashboard window should be active");
    assert_eq!(dashboard_win["spec"]["route"], "/dashboard/translator");

    // 验证历史错误调用的 kernel_open_window 不存在并被明确报错拒绝
    let bogus_resp = host.invoke_ipc(
        "kernel_open_window",
        json!({
            "spec": {
                "label": "dashboard",
                "route": "/overview",
                "title": "Dashboard",
            }
        }),
    );
    assert!(!bogus_resp.ok);
    assert!(bogus_resp
        .error
        .unwrap()
        .contains("Unknown IPC command: kernel_open_window"));
}
