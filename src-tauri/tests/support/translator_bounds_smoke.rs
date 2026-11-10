//! Windows 原生气泡定位检查：读取实际 HWND 对应窗口的位置与尺寸，不模拟 DPI 换算。
use super::*;

pub(super) fn check(app: &AppHandle, host: &Arc<DesktopHost>) -> anyhow::Result<()> {
    let monitor = app
        .available_monitors()?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("translator smoke: no monitor"))?;
    let scale = monitor.scale_factor();
    anyhow::ensure!(scale.is_finite() && scale > 0.0, "invalid monitor scale");
    let work = monitor.work_area();
    let preset = WindowSpec::translate_popover(0.0, 0.0);
    if f64::from(work.size.width) < preset.width * scale
        || f64::from(work.size.height) < preset.height * scale
    {
        println!("UI_SMOKE_NOTE: translator bounds skipped: work area smaller than popover");
        return Ok(());
    }

    // 与 Tauri 枚举顺序一致地选择第一块显示器，并在其物理右下角请求展开。
    let x = (f64::from(monitor.position().x) + f64::from(monitor.size().width) - 1.0) / scale;
    let y = (f64::from(monitor.position().y) + f64::from(monitor.size().height) - 1.0) / scale;
    let opened = host.invoke_ipc(
        "translator_open_popover",
        serde_json::json!({ "x": x, "y": y }),
    );
    anyhow::ensure!(opened.ok, "translator open: {:?}", opened.error);
    let window = app
        .get_webview_window("translate-popover")
        .ok_or_else(|| anyhow::anyhow!("translator native window missing"))?;
    let left = i64::from(work.position.x);
    let top = i64::from(work.position.y);
    let right = left + i64::from(work.size.width);
    let bottom = top + i64::from(work.size.height);
    let fits = wait_until(|| {
        window
            .outer_position()
            .ok()
            .zip(window.outer_size().ok())
            .is_some_and(|(p, s)| {
                i64::from(p.x) >= left - 1
                    && i64::from(p.y) >= top - 1
                    && i64::from(p.x) + i64::from(s.width) <= right + 1
                    && i64::from(p.y) + i64::from(s.height) <= bottom + 1
            })
    });
    let position = window.outer_position()?;
    let size = window.outer_size()?;
    anyhow::ensure!(fits, "translator outside work area: position={position:?}, size={size:?}, work={work:?}, scale={scale}");
    println!("UI_SMOKE_TRANSLATOR_BOUNDS: PASS position={position:?}, size={size:?}, work={work:?}, scale={scale}");
    let closed = host.invoke_ipc("translator_close_popover", serde_json::json!({}));
    anyhow::ensure!(closed.ok, "translator close: {:?}", closed.error);
    anyhow::ensure!(
        wait_until(|| app.get_webview_window("translate-popover").is_none()),
        "translator window cleanup"
    );
    Ok(())
}
