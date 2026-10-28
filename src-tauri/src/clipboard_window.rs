//! Keep Dashboard out of macOS's key-window fallback during external quick paste.
use tauri::{AppHandle, Manager};

pub(crate) fn prepare_popup(app: &AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let external = objc2_app_kit::NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .is_some_and(|front| front.processIdentifier() as u32 != std::process::id());
        if external {
            if let Some(dashboard) = app.get_webview_window("dashboard") {
                // Do not show it again on popup destruction: that would steal focus
                // or flash before the destination application is reactivated.
                dashboard.hide().map_err(|error| error.to_string())?;
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
    Ok(())
}
