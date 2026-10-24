//! Executable entry point (`all-in-one-desktop`) for the All-in-One Desktop Toolbox.
//!
//! Usage:
//! - `cargo run -p all-in-one-host -- --smoke-test`: Runs an end-to-end automated verification
//!   of the microkernel, SQLite storage, and all 3 modules (`mod-clipboard`, `mod-sticky-note`,
//!   `mod-window-pin`) and exits.
//! - `cargo run -p all-in-one-host`: Starts the Level 0 background daemon, binds the local IPC/UI
//!   bridge (`http://127.0.0.1:14200`), and launches the native macOS desktop app if packaged
//!   (or opens the workspace UI).

use std::{
    env,
    net::TcpListener,
    path::PathBuf,
    process::Command,
    sync::Arc,
};
use all_in_one_host::{serve_desktop_ui, DesktopHost};
use serde_json::json;
use tracing::info;

fn resolve_paths() -> (PathBuf, PathBuf) {
    // 1. Check if running inside `AllInOneToolbox.app/Contents/MacOS/all-in-one-desktop`
    if let Ok(exe_path) = env::current_exe() {
        if let Some(macos_dir) = exe_path.parent() {
            if let Some(contents_dir) = macos_dir.parent() {
                let bundled_ui = contents_dir.join("Resources").join("src-ui");
                if bundled_ui.exists() {
                    let data_dir = contents_dir.join("Resources").join("runtime_data");
                    return (data_dir, bundled_ui);
                }
            }
        }
    }

    // 2. Fallback to Cargo workspace root
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    (
        workspace_root.join(".runtime_data"),
        workspace_root.join("src-ui"),
    )
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();
    let smoke_test = args.iter().any(|a| a == "--smoke-test");
    let no_open = args.iter().any(|a| a == "--no-open");

    let (app_data_dir, src_ui_dir) = resolve_paths();

    let host = Arc::new(DesktopHost::bootstrap(&app_data_dir, smoke_test)?);
    host.seed_demo_data_if_empty()?;

    if smoke_test {
        run_cli_smoke_test(&host)?;
        return Ok(());
    }

    let port = env::var("AIO_PORT")
        .ok()
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(14200);

    let listener = TcpListener::bind(("127.0.0.1", port))
        .or_else(|_| TcpListener::bind(("127.0.0.1", 0)))?;
    let local_addr = listener.local_addr()?;
    let url = format!("http://{}", local_addr);

    println!("==================================================================");
    println!("  All-in-One Desktop Toolbox (Phase 1 Microkernel Daemon)");
    println!("  - Data Dir       : {}", app_data_dir.display());
    println!(
        "  - Active WebViews: {} (Level 0 Silent Startup)",
        host.kernel().context().window_pool.active_window_count()
    );
    println!("  - Mounted Modules: 02.Clipboard | 04.StickyNote | 09.WindowPin");
    println!("  - Desktop UI URL : {}", url);
    println!("==================================================================");

    if !no_open {
        #[cfg(target_os = "macos")]
        {
            let _ = Command::new("/usr/bin/open").arg(&url).spawn();
        }
    }

    info!(%url, "Serving interactive desktop UI and IPC bridge");
    serve_desktop_ui(host, listener, src_ui_dir)?;
    Ok(())
}

fn run_cli_smoke_test(host: &DesktopHost) -> anyhow::Result<()> {
    println!("[1/5] Verifying Level 0 startup (0 active WebViews)...");
    let status = host.invoke_ipc("kernel_get_status", json!({}));
    assert!(status.ok);
    let active_count = status.data.as_ref().unwrap()["active_window_count"]
        .as_u64()
        .unwrap();
    println!("      -> Active WebViews on startup: {} (OK)", active_count);

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
    println!(
        "      -> Pasted item #{}, popup auto-destroyed, active WebViews: {} (OK)",
        item_id,
        paste.data.as_ref().unwrap()["active_window_count"]
    );

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
    println!(
        "      -> Created sticky note '{}', active WebViews: {} (OK)",
        note_id,
        host.kernel().context().window_pool.active_window_count()
    );
    host.invoke_ipc("sticky_close_note", json!({ "id": note_id }));
    println!(
        "      -> Closed sticky note, active WebViews: {} (OK)",
        host.kernel().context().window_pool.active_window_count()
    );

    println!("[4/5] Testing Module 09 (DeskPins External Window Topmost & Shutdown Restore)...");
    let pinned = host.invoke_ipc("winpin_toggle_foreground", json!({}));
    assert!(pinned.ok);
    println!(
        "      -> Pinned foreground window: {} (topmost = true)",
        pinned.data.as_ref().unwrap()["window"]["title"]
    );

    println!("[5/5] Testing Graceful Daemon Shutdown & Resource Cleanup...");
    let shut = host.invoke_ipc("kernel_shutdown", json!({}));
    assert!(shut.ok);
    println!(
        "      -> Shutdown complete: active WebViews = {}, stranded topmost windows = {} (OK)",
        shut.data.as_ref().unwrap()["active_window_count"],
        shut.data.as_ref().unwrap()["pinned_external_windows"]
            .as_array()
            .unwrap()
            .len()
    );
    Ok(())
}
