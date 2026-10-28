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
fn sticky_ipc_keeps_latest_state_when_close_follows_a_stale_write() {
    let dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
    let created = host.invoke_ipc(
        "sticky_create_note",
        json!({
            "title": "旧标题",
            "content_md": "旧正文",
            "theme_color": "amber"
        }),
    );
    assert!(created.ok, "{:?}", created.error);
    let id = created.data.unwrap()["id"].as_str().unwrap().to_string();

    let newer = host.invoke_ipc(
        "sticky_save_content",
        json!({
            "id": id,
            "title": "新标题",
            "content_md": "新正文",
            "write_seq": 2
        }),
    );
    assert!(newer.ok, "{:?}", newer.error);
    assert_eq!(newer.data.unwrap()["applied"], true);
    let older = host.invoke_ipc(
        "sticky_save_content",
        json!({
            "id": id,
            "title": "回退标题",
            "content_md": "回退正文",
            "write_seq": 1
        }),
    );
    assert!(older.ok, "{:?}", older.error);
    let older_note = older.data.unwrap();
    assert_eq!(older_note["applied"], false);
    assert_eq!(older_note["content_md"], "新正文");

    let theme = host.invoke_ipc(
        "sticky_update_theme",
        json!({ "id": id, "theme_color": "rose", "write_seq": 3 }),
    );
    assert!(theme.ok, "{:?}", theme.error);
    let stale_theme = host.invoke_ipc(
        "sticky_update_theme",
        json!({ "id": id, "theme_color": "sky", "write_seq": 1 }),
    );
    assert!(stale_theme.ok, "{:?}", stale_theme.error);
    let stale_theme_note = stale_theme.data.unwrap();
    assert_eq!(stale_theme_note["applied"], false);
    assert_eq!(stale_theme_note["theme_color"], "rose");

    let window = host.invoke_ipc(
        "sticky_update_window_state",
        json!({ "id": id, "opacity": 0.5, "is_pinned": false, "write_seq": 5 }),
    );
    assert!(window.ok, "{:?}", window.error);
    let stale_window = host.invoke_ipc(
        "sticky_update_window_state",
        json!({ "id": id, "opacity": 0.75, "is_pinned": true, "write_seq": 4 }),
    );
    assert!(stale_window.ok, "{:?}", stale_window.error);
    let stale_window_note = stale_window.data.unwrap();
    assert_eq!(stale_window_note["applied"], false);
    assert_eq!(stale_window_note["is_pinned"], false);

    let missing = host.invoke_ipc(
        "sticky_save_content",
        json!({
            "id": "missing-note",
            "title": "x",
            "content_md": "y",
            "write_seq": 1
        }),
    );
    assert!(!missing.ok);

    let closed = host.invoke_ipc("sticky_request_close", json!({ "id": id }));
    assert!(closed.ok, "{:?}", closed.error);
    let closed_data = closed.data.unwrap();
    assert_eq!(closed_data["delegated"], false);
    assert_eq!(closed_data["closed"], true);
    assert_eq!(closed_data["active_window_count"], 0);

    let stored = host.invoke_ipc("sticky_get_note", json!({ "id": id }));
    assert!(stored.ok, "{:?}", stored.error);
    let note = stored.data.unwrap();
    assert_eq!(note["title"], "新标题");
    assert_eq!(note["content_md"], "新正文");
    assert_eq!(note["theme_color"], "rose");
    assert_eq!(note["opacity"], 0.5);
    assert_eq!(note["is_pinned"], false);
    assert_eq!(note["is_open"], false);
}

#[test]
fn sticky_ipc_reopen_accepts_a_new_session_and_ignores_the_closed_webview() {
    let dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
    let created = host.invoke_ipc(
        "sticky_create_note",
        json!({
            "title": "旧标题",
            "content_md": "旧正文",
            "theme_color": "amber"
        }),
    );
    assert!(created.ok, "{:?}", created.error);
    let id = created.data.unwrap()["id"].as_str().unwrap().to_string();

    let first = host.invoke_ipc("sticky_begin_session", json!({ "id": id }));
    assert!(first.ok, "{:?}", first.error);
    let first_session = first.data.unwrap()["session_id"].as_u64().unwrap();
    assert!(
        host.invoke_ipc(
            "sticky_save_content",
            json!({
                "id": id,
                "title": "第一轮标题",
                "content_md": "第一轮正文",
                "write_seq": 5,
                "session_id": first_session
            }),
        )
        .ok
    );
    assert!(
        host.invoke_ipc(
            "sticky_update_theme",
            json!({
                "id": id,
                "theme_color": "rose",
                "write_seq": 4,
                "session_id": first_session
            }),
        )
        .ok
    );
    assert!(
        host.invoke_ipc(
            "sticky_update_window_state",
            json!({
                "id": id,
                "opacity": 0.5,
                "is_pinned": true,
                "write_seq": 6,
                "session_id": first_session
            }),
        )
        .ok
    );

    let closed = host.invoke_ipc("sticky_close_note", json!({ "id": id }));
    assert!(closed.ok, "{:?}", closed.error);
    let opened = host.invoke_ipc("sticky_open_note", json!({ "id": id }));
    assert!(opened.ok, "{:?}", opened.error);

    let second = host.invoke_ipc("sticky_begin_session", json!({ "id": id }));
    assert!(second.ok, "{:?}", second.error);
    let second_session = second.data.unwrap()["session_id"].as_u64().unwrap();
    assert_ne!(first_session, second_session);

    let fresh = host.invoke_ipc(
        "sticky_save_content",
        json!({
            "id": id,
            "title": "第二轮标题",
            "content_md": "第二轮正文",
            "write_seq": 1,
            "session_id": second_session
        }),
    );
    assert!(fresh.ok, "{:?}", fresh.error);
    let fresh_note = fresh.data.unwrap();
    assert_eq!(fresh_note["applied"], true);
    assert_eq!(fresh_note["title"], "第二轮标题");
    assert!(
        host.invoke_ipc(
            "sticky_update_theme",
            json!({
                "id": id,
                "theme_color": "violet",
                "write_seq": 1,
                "session_id": second_session
            }),
        )
        .ok
    );
    assert!(
        host.invoke_ipc(
            "sticky_update_window_state",
            json!({
                "id": id,
                "opacity": 0.25,
                "is_pinned": false,
                "write_seq": 1,
                "session_id": second_session
            }),
        )
        .ok
    );

    let delayed = host.invoke_ipc(
        "sticky_save_content",
        json!({
            "id": id,
            "title": "过期标题",
            "content_md": "过期正文",
            "write_seq": 99,
            "session_id": first_session
        }),
    );
    assert!(delayed.ok, "{:?}", delayed.error);
    let delayed_note = delayed.data.unwrap();
    assert_eq!(delayed_note["applied"], false);
    assert_eq!(delayed_note["content_md"], "第二轮正文");
    let delayed_theme = host.invoke_ipc(
        "sticky_update_theme",
        json!({
            "id": id,
            "theme_color": "sky",
            "write_seq": 99,
            "session_id": first_session
        }),
    );
    assert!(delayed_theme.ok, "{:?}", delayed_theme.error);
    let delayed_theme_note = delayed_theme.data.unwrap();
    assert_eq!(delayed_theme_note["applied"], false);
    assert_eq!(delayed_theme_note["theme_color"], "violet");
    let delayed_window = host.invoke_ipc(
        "sticky_update_window_state",
        json!({
            "id": id,
            "opacity": 0.75,
            "is_pinned": true,
            "write_seq": 99,
            "session_id": first_session
        }),
    );
    assert!(delayed_window.ok, "{:?}", delayed_window.error);
    let delayed_window_note = delayed_window.data.unwrap();
    assert_eq!(delayed_window_note["applied"], false);
    assert_eq!(delayed_window_note["is_pinned"], false);
    let unscoped = host.invoke_ipc(
        "sticky_save_content",
        json!({
            "id": id,
            "title": "无会话标题",
            "content_md": "无会话正文",
            "write_seq": 100
        }),
    );
    assert!(unscoped.ok, "{:?}", unscoped.error);
    let unscoped_note = unscoped.data.unwrap();
    assert_eq!(unscoped_note["applied"], false);
    assert_eq!(unscoped_note["content_md"], "第二轮正文");

    let stored = host.invoke_ipc("sticky_get_note", json!({ "id": id }));
    assert!(stored.ok, "{:?}", stored.error);
    let note = stored.data.unwrap();
    assert_eq!(note["title"], "第二轮标题");
    assert_eq!(note["content_md"], "第二轮正文");
    assert_eq!(note["theme_color"], "violet");
    assert_eq!(note["opacity"], 0.25);
    assert_eq!(note["is_pinned"], false);

    let missing = host.invoke_ipc("sticky_begin_session", json!({ "id": "missing-note" }));
    assert!(!missing.ok);
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
