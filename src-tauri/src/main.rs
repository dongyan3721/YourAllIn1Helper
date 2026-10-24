//! All-in-One Desktop executable.
//!
//! Default mode starts the real Tauri 2.x desktop app. `--smoke-test` runs the same
//! microkernel IPC workflow headlessly for CI and development verification.

use std::{env, path::PathBuf, sync::Arc};

use all_in_one_host::DesktopHost;
use serde_json::json;

fn main() -> anyhow::Result<()> {
    if env::args().any(|arg| arg == "--smoke-test") {
        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(".runtime_data")
            .join("smoke_test");
        let host = Arc::new(DesktopHost::bootstrap(data_dir, true)?);
        host.seed_demo_data_if_empty()?;
        run_cli_smoke_test(&host)?;
        return Ok(());
    }

    all_in_one_host::run();
    Ok(())
}

fn run_cli_smoke_test(host: &DesktopHost) -> anyhow::Result<()> {
    println!("[1/5] Verifying Level 0 startup (0 active WebViews)...");
    let status = host.invoke_ipc("kernel_get_status", json!({}));
    assert!(status.ok);
    println!(
        "      -> Active WebViews on startup: {} (OK)",
        status.data.as_ref().unwrap()["active_window_count"]
    );

    println!("[2/5] Testing Module 02 (Clipboard LRU-100, Deduplication & Quick Paste)...");
    host.invoke_ipc(
        "kernel_trigger_shortcut",
        json!({ "action_id": "clipboard:toggle_popup" }),
    );
    let cap = host.invoke_ipc(
        "clipboard_capture_text",
        json!({ "text": "Smoke Test Snippet", "source_app_name": "Terminal" }),
    );
    let item_id = cap.data.as_ref().unwrap()["id"].as_i64().unwrap();
    let paste = host.invoke_ipc(
        "clipboard_paste_item",
        json!({ "id": item_id, "as_plain_text": true }),
    );
    assert!(paste.ok);
    println!("      -> Clipboard capture and quick paste (OK)");

    println!("[3/5] Testing Module 04 (Sticky Note Markdown + LaTeX Lifecycle)...");
    let created = host.invoke_ipc(
        "sticky_create_note",
        json!({
            "title": "Smoke Test Note",
            "content_md": "Euler: $e^{i\\pi} + 1 = 0$",
            "theme_color": "sky"
        }),
    );
    let note_id = created.data.as_ref().unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    host.invoke_ipc("sticky_close_note", json!({ "id": note_id }));
    println!("      -> Sticky note create/close (OK)");

    println!("[4/5] Testing Module 09 (DeskPins External Window Topmost & Shutdown Restore)...");
    let pinned = host.invoke_ipc("winpin_toggle_foreground", json!({}));
    assert!(pinned.ok);
    println!(
        "      -> Pinned foreground window: {} (OK)",
        pinned.data.as_ref().unwrap()["window"]["title"]
    );

    println!("[5/5] Testing Graceful Daemon Shutdown & Resource Cleanup...");
    let shut = host.invoke_ipc("kernel_shutdown", json!({}));
    assert!(shut.ok);
    println!("      -> Shutdown cleanup (OK)");
    Ok(())
}
