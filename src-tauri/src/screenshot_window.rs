//! 截图遮罩的原生层级策略；领域模块不依赖 AppKit。
//!
//! 遮罩的标注工具栏、手柄与放大镜都是同一 WebView 内的 DOM 元素，由 CSS 层叠保证位于画布之上，
//! 不存在屏幕注释那种「两个同层原生窗口互相重排」的问题。这里只负责遮罩窗口本身：
//! macOS 普通 always_on_top 为 floating 层，低于菜单栏与 Dock，且与贴图窗口同层，
//! 因此显式提升到菜单栏之上，并允许出现在全屏应用所在的空间。

use app_core::{CoreError, CoreResult};
use tauri::{AppHandle, Manager, WebviewWindow};

/// 显示遮罩并尽量取得键盘焦点。层级提升失败时返回错误，由调用方销毁遮罩。
/// 焦点失败不阻断：主窗口最小化时系统可能拒绝把键焦点交给新窗口，但遮罩已经置于前台。
pub(crate) fn present_capture_overlay(app: &AppHandle, overlay: &WebviewWindow) -> CoreResult<()> {
    overlay.show().map_err(to_core_error)?;
    elevate_overlay(app, overlay)?;
    let _ = overlay.set_focus();
    Ok(())
}

#[cfg(target_os = "macos")]
fn elevate_overlay(app: &AppHandle, overlay: &WebviewWindow) -> CoreResult<()> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{
        NSApplication, NSMainMenuWindowLevel, NSWindow, NSWindowCollectionBehavior,
    };

    let overlay = overlay.clone();
    let app_handle = app.clone();
    let apply = move || -> CoreResult<()> {
        // SAFETY: 窗口由 Tauri 持有，闭包只在 AppKit 主线程执行。
        let window: &NSWindow = unsafe {
            &*overlay
                .ns_window()
                .map_err(to_core_error)?
                .cast::<NSWindow>()
        };
        // 高于菜单栏（24）与状态栏（25），仍低于输入法候选与系统弹出菜单，文字标注可正常输入。
        window.setLevel(NSMainMenuWindowLevel + 2);
        window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        window.setHasShadow(false);
        // 主面板缩在 Dock 里时应用是非激活的。激活应用才能把遮罩置于前台，
        // 但激活有时会把已最小化的主面板一起拉回来，挡住选区和滚动预览。先记下状态，激活后再缩回去。
        let dashboard_was_miniaturized = ns_window_is_miniaturized(&app_handle, "dashboard");
        if let Some(mtm) = MainThreadMarker::new() {
            let ns_app = NSApplication::sharedApplication(mtm);
            if ns_app.isHidden() {
                ns_app.unhide(None);
            }
            // 新的 activate() 不保证抢到前台。截图热键必须盖住当前应用。
            #[allow(deprecated)]
            ns_app.activateIgnoringOtherApps(true);
        }
        if dashboard_was_miniaturized {
            miniaturize_window(&app_handle, "dashboard");
        }
        if window.isMiniaturized() {
            window.deminiaturize(None);
        }
        window.orderFrontRegardless();
        window.makeKeyAndOrderFront(None);
        if overlay.label() == mod_screenshot::OVERLAY_LABEL {
            let window_id = window.windowNumber() as u32;
            if let Some(host) = app_handle.try_state::<std::sync::Arc<crate::DesktopHost>>() {
                host.screenshot().set_overlay_window_id(window_id);
            }
        }
        Ok(())
    };
    if MainThreadMarker::new().is_some() {
        return apply();
    }
    // 等待层级真正生效再返回，确保「遮罩可见」时已在菜单栏与贴图之上。
    let (sender, receiver) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = sender.send(apply());
    })
    .map_err(to_core_error)?;
    receiver
        .recv_timeout(std::time::Duration::from_secs(5))
        .map_err(to_core_error)?
}

/// Windows 下 always_on_top 即 HWND_TOPMOST，后创建的遮罩位于任务栏与贴图之上；
/// Linux 不在本项目验收范围内。
#[cfg(not(target_os = "macos"))]
fn elevate_overlay(_app: &AppHandle, _overlay: &WebviewWindow) -> CoreResult<()> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn ns_window_is_miniaturized(app: &AppHandle, label: &str) -> bool {
    let Some(window) = app.get_webview_window(label) else {
        return false;
    };
    let Ok(ptr) = window.ns_window() else {
        return false;
    };
    // SAFETY: 只在 AppKit 主线程读取窗口状态，指针由 Tauri 持有。
    unsafe { (*ptr.cast::<objc2_app_kit::NSWindow>()).isMiniaturized() }
}

#[cfg(target_os = "macos")]
fn miniaturize_window(app: &AppHandle, label: &str) {
    let Some(window) = app.get_webview_window(label) else {
        return;
    };
    let Ok(ptr) = window.ns_window() else {
        return;
    };
    // SAFETY: 调用方已在 AppKit 主线程。
    let window = unsafe { &*ptr.cast::<objc2_app_kit::NSWindow>() };
    if !window.isMiniaturized() {
        window.miniaturize(None);
    }
}

fn to_core_error(error: impl std::fmt::Display) -> CoreError {
    CoreError::Internal(anyhow::anyhow!(error.to_string()))
}
