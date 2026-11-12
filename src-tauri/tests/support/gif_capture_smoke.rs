//! Real WebView2 animation -> native overlay hole -> GIF -> decoded moving frames.
//! Uses only isolated QA data and windows created by this process.
use super::*;

pub(super) fn check(app: &AppHandle, host: &Arc<DesktopHost>) -> anyhow::Result<()> {
    use serde_json::json;
    let (created_tx, created_rx) = std::sync::mpsc::channel();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let result = tauri::WebviewWindowBuilder::new(
            &handle,
            "gif-qa-target",
            tauri::WebviewUrl::App("index.html".into()),
        )
        .on_page_load(move |_, page| {
            if page.event() == tauri::webview::PageLoadEvent::Finished {
                let _ = ready_tx.send(());
            }
        })
        .title("GIF animation QA")
        .inner_size(420.0, 300.0)
        .position(200.0, 180.0)
        .decorations(false)
        .always_on_top(true)
        .build()
        .map_err(anyhow::Error::from);
        let _ = created_tx.send(result);
    })?;
    let target = created_rx.recv_timeout(Duration::from_secs(10))??;
    let result = (|| -> anyhow::Result<()> {
        ready_rx.recv_timeout(Duration::from_secs(15))?;
        // The native page-load callback avoids granting JS event permissions to a QA window.
        target.eval(r#"
            document.body.innerHTML = '<div id="gif-qa" style="position:fixed;inset:0;z-index:2147483647;background:#f02020"></div>';
            let tick = 0;
            setInterval(() => { document.getElementById('gif-qa').style.background = (++tick % 2) ? '#2050f0' : '#f02020'; }, 200);
        "#)?;
        std::thread::sleep(Duration::from_millis(400));
        let position = target.inner_position()?;
        let size = target.inner_size()?;
        let started = host.invoke_ipc("screenshot_start_capture", json!({"hide_dashboard":true}));
        anyhow::ensure!(started.ok, "GIF QA capture: {:?}", started.error);
        let info = started.data.unwrap();
        let id = info["session_id"].as_str().unwrap();
        let overlay = app
            .get_webview_window(mod_screenshot::OVERLAY_LABEL)
            .ok_or_else(|| anyhow::anyhow!("overlay missing"))?;
        anyhow::ensure!(
            wait_until(|| overlay.is_visible().unwrap_or(false)),
            "overlay ready"
        );
        // Pixel coordinates come from the actual native windows, not assumed logical DPI.
        let origin = overlay.inner_position()?;
        let fullscreen = std::env::var_os("ALL_IN_ONE_GIF_SMOKE_FULLSCREEN").is_some();
        let selection = if fullscreen {
            app_core::SelectionBox {
                x: 0,
                y: 0,
                width: info["frame_width"].as_u64().unwrap() as u32,
                height: info["frame_height"].as_u64().unwrap() as u32,
            }
        } else {
            app_core::SelectionBox {
                x: position.x - origin.x + 20,
                y: position.y - origin.y + 20,
                width: size.width.min(360) - 40,
                height: size.height.min(240) - 40,
            }
        };
        let recorded = host.invoke_ipc(
            "screenshot_gif_start",
            json!({"session_id":id,"selection":selection,"fps":10,"preset":if fullscreen {"480p"} else {"original"}}),
        );
        anyhow::ensure!(recorded.ok, "GIF QA recording: {:?}", recorded.error);
        // Independently inspect Win32 clipping and remote-capture affinity.
        unsafe {
            use windows::Win32::{
                Graphics::Gdi::{CreateRectRgn, DeleteObject, GetWindowRgn, PtInRegion, RGN_ERROR},
                UI::WindowsAndMessaging::GetWindowDisplayAffinity,
            };
            let hwnd = overlay.hwnd()?;
            let mut affinity = 99;
            GetWindowDisplayAffinity(hwnd, &mut affinity)?;
            anyhow::ensure!(
                affinity == 0,
                "GIF recording must not hide UI from remote screen capture"
            );
            let region = CreateRectRgn(0, 0, 0, 0);
            let kind = GetWindowRgn(hwnd, region);
            let clipped = !PtInRegion(region, selection.x + 10, selection.y + 10).as_bool();
            let _ = DeleteObject(region.into());
            anyhow::ensure!(
                kind != RGN_ERROR && clipped,
                "native recording hole missing"
            );
        }
        std::thread::sleep(Duration::from_secs(3));
        host.kernel()
            .trigger_action(mod_screenshot::ACTION_CAPTURE)?;
        anyhow::ensure!(
            host.screenshot()
                .session_info(host.kernel().context(), None)?
                .session_id
                == id,
            "GIF stop must preserve the original capture session"
        );
        anyhow::ensure!(
            wait_until(|| host
                .screenshot()
                .gif_status()
                .is_some_and(|s| !s.is_recording)),
            "GIF encoder completion"
        );
        let bytes = host.screenshot_gif_bytes()?;
        let options = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(&bytes))?;
        use image::AnimationDecoder;
        let frames = options.into_frames().collect_frames()?;
        let mut red = 0;
        let mut blue = 0;
        for frame in &frames {
            let source_x = (position.x - origin.x + 40 - selection.x) as u32;
            let source_y = (position.y - origin.y + 40 - selection.y) as u32;
            let p = frame.buffer().get_pixel(
                source_x * frame.buffer().width() / selection.width,
                source_y * frame.buffer().height() / selection.height,
            );
            if p[0] > 150 && p[2] < 100 {
                red += 1;
            }
            if p[2] > 150 && p[0] < 100 {
                blue += 1;
            }
        }
        anyhow::ensure!(
            frames.len() >= 5 && red > 0 && blue > 0,
            "GIF must contain changing desktop pixels, frames={}, red={red}, blue={blue}",
            frames.len()
        );
        let artifact = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(if fullscreen {
            "../.runtime_data/gif-native-fullscreen-qa.gif"
        } else {
            "../.runtime_data/gif-native-qa.gif"
        });
        std::fs::write(&artifact, bytes)?;
        println!(
            "UI_SMOKE_GIF: PASS frames={}, red={red}, blue={blue}, WDA_NONE, native hole; {}",
            frames.len(),
            artifact.display()
        );
        let cancelled = host.invoke_ipc("screenshot_gif_cancel", json!({}));
        anyhow::ensure!(cancelled.ok, "GIF QA cleanup: {:?}", cancelled.error);
        Ok(())
    })();
    let _ = host.invoke_ipc("screenshot_cancel", json!({}));
    if result.is_ok() && std::env::var_os("ALL_IN_ONE_GIF_GUI_QA").is_some() {
        host.open_feature_panel("screenshot")?;
    } else {
        let _ = target.destroy();
    }
    result
}
