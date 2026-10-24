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
    assert_eq!(data["modules"].as_array().unwrap().len(), 3);

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
