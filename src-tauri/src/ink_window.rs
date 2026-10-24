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

#[cfg(target_os = "macos")]
fn internal(error: impl std::fmt::Display) -> CoreError {
    CoreError::Internal(anyhow::anyhow!(error.to_string()))
}
