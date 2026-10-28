//! 屏幕注释的原生窗口层级策略；领域模块不依赖 AppKit。

#[cfg(target_os = "macos")]
use app_core::CoreError;
use app_core::CoreResult;
use tauri::{AppHandle, WebviewWindow};

/// macOS 的普通 floating 层内仍会重排窗口，工具栏必须占据画布之上的独立层。
#[cfg(target_os = "macos")]
pub(crate) fn elevate_toolbar(app: &AppHandle, toolbar: &WebviewWindow) -> CoreResult<()> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSWindow, NSWindowOrderingMode};
    use tauri::Manager;

    let canvas = app.get_webview_window("screen-ink-canvas").ok_or_else(|| {
        CoreError::Config("Screen ink canvas is not available for toolbar ordering".into())
    })?;
    let toolbar = toolbar.clone();
    let apply = move || -> CoreResult<()> {
        // SAFETY: 两个窗口由 Tauri 持有，且本闭包只在 AppKit 主线程执行。
        let canvas: &NSWindow =
            unsafe { &*canvas.ns_window().map_err(internal)?.cast::<NSWindow>() };
        let toolbar: &NSWindow =
            unsafe { &*toolbar.ns_window().map_err(internal)?.cast::<NSWindow>() };
        // AppKit 在 orderOut / show 后可能解除 child 关系；截图恢复时重新绑定。
        if !toolbar
            .parentWindow()
            .is_some_and(|parent| std::ptr::eq(&*parent, canvas))
        {
            // SAFETY: 两个窗口均存活且位于同一 AppKit 主线程，子窗口排在父窗口上方。
            unsafe { canvas.addChildWindow_ordered(toolbar, NSWindowOrderingMode::Above) };
        }
        toolbar.setLevel(canvas.level() + 1);
        toolbar.orderFrontRegardless();
        Ok(())
    };

    if MainThreadMarker::new().is_some() {
        return apply();
    }
    // 等待原生层级实际应用，避免启动成功事件早于工具栏可交互；主线程分支直接执行。
    let (sender, receiver) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = sender.send(apply());
    })
    .map_err(internal)?;
    receiver
        .recv_timeout(std::time::Duration::from_secs(5))
        .map_err(internal)?
}

/// Windows / Linux 使用 Tauri parent 对应的 owner / transient 层级约束。
#[cfg(not(target_os = "macos"))]
pub(crate) fn elevate_toolbar(_app: &AppHandle, _toolbar: &WebviewWindow) -> CoreResult<()> {
    Ok(())
}

/// 立刻把屏幕注释画布的鼠标改成指定形态。
///
/// AppKit 的 `invalidateCursorRects` 只在下一次鼠标移动或点击时才换光标，指针停在画布上时
/// 会一直保持旧形态。这里在主线程直接 `[NSCursor set]`，并在会话期间暂停画布的 cursor rects，
/// 避免 WebKit 用过期矩形把刚设置的光标改回去。穿透或会话结束时交还系统箭头。
/// 光标刷新失败不向外报错，避免拖垮穿透切换本身。
pub(crate) fn apply_cursor_shape(app: &AppHandle, shape: &str) {
    #[cfg(target_os = "macos")]
    apply_cursor_shape_macos(app, shape);
    #[cfg(not(target_os = "macos"))]
    apply_cursor_shape_other(app, shape);
}

#[cfg(target_os = "macos")]
fn apply_cursor_shape_macos(app: &AppHandle, shape: &str) {
    use objc2::MainThreadMarker;

    let shape = normalize_cursor_shape(shape).to_string();
    let app = app.clone();
    let queued_app = app.clone();
    let queued_shape = shape.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    // 推迟到当前鼠标事件结束之后。工具栏点击或快捷键回调还在处理时直接 set，
    // AppKit 会在事件收尾按旧的 cursor rects 把光标改回去。
    let queued = app.run_on_main_thread(move || {
        apply_cursor_shape_on_main(&queued_app, &queued_shape);
        let _ = sender.send(());
    });
    if queued.is_err() {
        if MainThreadMarker::new().is_some() {
            apply_cursor_shape_on_main(&app, &shape);
        } else {
            tracing::warn!("screen ink cursor update could not reach the AppKit main thread");
        }
        return;
    }
    if MainThreadMarker::new().is_some() {
        return;
    }
    if receiver
        .recv_timeout(std::time::Duration::from_millis(500))
        .is_err()
    {
        tracing::warn!("screen ink cursor update timed out");
    }
}

#[cfg(target_os = "macos")]
fn apply_cursor_shape_on_main(app: &AppHandle, shape: &str) {
    use std::sync::atomic::{AtomicBool, Ordering};

    use objc2_app_kit::{NSCursor, NSWindow};
    use tauri::Manager;

    /// 只记录当前这扇画布是否已暂停 cursor rects。窗口销毁后清零，下一扇新窗口会重新暂停。
    static CURSOR_RECTS_DISABLED: AtomicBool = AtomicBool::new(false);

    let Some(canvas) = app.get_webview_window("screen-ink-canvas") else {
        CURSOR_RECTS_DISABLED.store(false, Ordering::SeqCst);
        NSCursor::arrowCursor().set();
        return;
    };
    let Ok(pointer) = canvas.ns_window() else {
        NSCursor::arrowCursor().set();
        return;
    };
    // SAFETY: 画布窗口由 Tauri 持有，且本函数只在 AppKit 主线程执行。
    let window: &NSWindow = unsafe { &*pointer.cast::<NSWindow>() };
    // 会话存续期间一直暂停这扇窗口的 cursor rects，避免 WebKit 在 set 之后把旧形态写回去。
    // 穿透时窗口已不参与命中测试，下方应用仍会在鼠标移动时换上自己的光标。
    if !CURSOR_RECTS_DISABLED.swap(true, Ordering::SeqCst) {
        window.disableCursorRects();
    }
    cursor_for_shape(shape).set();
}

#[cfg(target_os = "macos")]
fn cursor_for_shape(shape: &str) -> objc2::rc::Retained<objc2_app_kit::NSCursor> {
    use objc2_app_kit::NSCursor;

    match shape {
        "arrow" => NSCursor::arrowCursor(),
        "cell" => custom_cursor(cell_png(), 8.0, 8.0).unwrap_or_else(NSCursor::crosshairCursor),
        "none" => custom_cursor(clear_png(), 0.0, 0.0).unwrap_or_else(NSCursor::arrowCursor),
        _ => NSCursor::crosshairCursor(),
    }
}

#[cfg(target_os = "macos")]
fn custom_cursor(
    png: &[u8],
    hot_x: f64,
    hot_y: f64,
) -> Option<objc2::rc::Retained<objc2_app_kit::NSCursor>> {
    use objc2::AnyThread;
    use objc2_app_kit::{NSCursor, NSImage};
    use objc2_foundation::{NSData, NSPoint};

    if png.is_empty() {
        return None;
    }
    // dataWithBytes 会复制字节，png 只需要在这次调用期间有效。
    let data = unsafe { NSData::dataWithBytes_length(png.as_ptr().cast(), png.len()) };
    let image = NSImage::initWithData(NSImage::alloc(), &data)?;
    Some(NSCursor::initWithImage_hotSpot(
        NSCursor::alloc(),
        &image,
        NSPoint { x: hot_x, y: hot_y },
    ))
}

#[cfg(target_os = "macos")]
fn cell_png() -> &'static [u8] {
    use std::sync::OnceLock;

    static PNG: OnceLock<Vec<u8>> = OnceLock::new();
    PNG.get_or_init(|| {
        let mut pixels = vec![0u8; 16 * 16 * 4];
        for y in 0..16u32 {
            for x in 0..16u32 {
                let dx = x as i32 - 8;
                let dy = y as i32 - 8;
                let span = dx.abs() <= 6 && dy.abs() <= 6;
                let halo = span && (dx.abs() <= 2 || dy.abs() <= 2);
                let core = span && (dx.abs() <= 1 || dy.abs() <= 1);
                let px = if core {
                    [20, 20, 20, 255]
                } else if halo {
                    [255, 255, 255, 230]
                } else {
                    [0, 0, 0, 0]
                };
                let index = ((y * 16 + x) * 4) as usize;
                pixels[index..index + 4].copy_from_slice(&px);
            }
        }
        encode_png(16, 16, &pixels)
    })
    .as_slice()
}

#[cfg(target_os = "macos")]
fn clear_png() -> &'static [u8] {
    use std::sync::OnceLock;

    static PNG: OnceLock<Vec<u8>> = OnceLock::new();
    PNG.get_or_init(|| encode_png(1, 1, &[0, 0, 0, 0]))
        .as_slice()
}

#[cfg(target_os = "macos")]
fn encode_png(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    let mut png = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut png);
    if image::ImageEncoder::write_image(
        encoder,
        pixels,
        width,
        height,
        image::ExtendedColorType::Rgba8,
    )
    .is_err()
    {
        return Vec::new();
    }
    png
}

#[cfg(not(target_os = "macos"))]
fn apply_cursor_shape_other(app: &AppHandle, shape: &str) {
    use tauri::{CursorIcon, Manager};

    let Some(window) = app.get_webview_window("screen-ink-canvas") else {
        return;
    };
    match normalize_cursor_shape(shape) {
        "none" => {
            let _ = window.set_cursor_visible(false);
        }
        "arrow" => {
            let _ = window.set_cursor_visible(true);
            let _ = window.set_cursor_icon(CursorIcon::Default);
        }
        "cell" => {
            let _ = window.set_cursor_visible(true);
            let _ = window.set_cursor_icon(CursorIcon::Cell);
        }
        _ => {
            let _ = window.set_cursor_visible(true);
            let _ = window.set_cursor_icon(CursorIcon::Crosshair);
        }
    }
}

fn normalize_cursor_shape(shape: &str) -> &'static str {
    match shape {
        "arrow" | "crosshair" | "cell" | "none" => shape_static(shape),
        _ => "crosshair",
    }
}

fn shape_static(shape: &str) -> &'static str {
    match shape {
        "arrow" => "arrow",
        "cell" => "cell",
        "none" => "none",
        _ => "crosshair",
    }
}

#[cfg(target_os = "macos")]
fn internal(error: impl std::fmt::Display) -> CoreError {
    CoreError::Internal(anyhow::anyhow!(error.to_string()))
}
