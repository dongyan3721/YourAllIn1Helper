//! Host-owned native media pickers. Feature modules never depend on UI frameworks.
//!
//! macOS presents an AppKit sheet on the calling panel's real NSWindow. The sheet uses a
//! completion handler, so neither the AppKit event loop nor the async IPC task is blocked.
//! Windows uses an owned IFileOpenDialog on a dedicated COM STA thread.

use crate::{DesktopHost, IpcResponse};
use app_core::{CoreError, CoreResult};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::AppHandle;

pub(crate) async fn pick_media(
    app: &AppHandle,
    host: &Arc<DesktopHost>,
    directory: bool,
    owner_label: &str,
) -> IpcResponse {
    let result = pick_media_result(app, host, directory, owner_label).await;
    match result {
        Ok(paths) => IpcResponse::success(selection_payload(directory, &paths)),
        Err(error) => IpcResponse::err(error.to_string()),
    }
}

async fn pick_media_result(
    app: &AppHandle,
    host: &Arc<DesktopHost>,
    directory: bool,
    owner_label: &str,
) -> CoreResult<Vec<PathBuf>> {
    ensure_media_enabled(host)?;
    let paths = platform::pick(app, host, directory, owner_label).await?;
    // A module can be disabled while a non-modal picker is open. Never return
    // selection results to a feature that was disabled in the meantime.
    ensure_media_enabled(host)?;
    Ok(paths)
}

fn ensure_media_enabled(host: &DesktopHost) -> CoreResult<()> {
    host.kernel()
        .registry()
        .with_enabled("media_proc", || Ok(()))
}

fn selection_payload(directory: bool, paths: &[PathBuf]) -> Value {
    if directory {
        serde_json::to_value(paths.first()).unwrap_or(Value::Null)
    } else {
        serde_json::to_value(paths).unwrap_or(Value::Null)
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use block2::RcBlock;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSModalResponseCancel, NSModalResponseOK, NSOpenPanel, NSWindow};
    use objc2_foundation::NSString;
    use std::sync::Mutex;
    use tauri::Manager;
    use tokio::sync::oneshot;

    pub(super) async fn pick(
        app: &AppHandle,
        _host: &Arc<DesktopHost>,
        directory: bool,
        owner_label: &str,
    ) -> CoreResult<Vec<PathBuf>> {
        let (sender, receiver) = oneshot::channel();
        let app = app.clone();
        let owner_label = owner_label.to_owned();
        app.clone()
            .run_on_main_thread(move || {
                if let Err(error) = present_dashboard_sheet(&app, directory, &owner_label, sender) {
                    tracing::error!(%error, "Failed to present native media picker");
                }
            })
            .map_err(|error| CoreError::Internal(anyhow::anyhow!(error.to_string())))?;
        receiver.await.map_err(|_| {
            CoreError::Config("native media picker closed without returning a result".into())
        })?
    }

    fn present_dashboard_sheet(
        app: &AppHandle,
        directory: bool,
        owner_label: &str,
        sender: oneshot::Sender<CoreResult<Vec<PathBuf>>>,
    ) -> CoreResult<()> {
        let mtm = MainThreadMarker::new().ok_or_else(|| {
            CoreError::Config("native media picker must run on the AppKit main thread".into())
        })?;
        let window = app
            .get_webview_window(owner_label)
            .ok_or_else(|| CoreError::Config("Owner window is not available".into()))?;
        window.show().map_err(internal)?;
        window.set_focus().map_err(internal)?;
        let raw_parent = window.ns_window().map_err(internal)?;
        if raw_parent.is_null() {
            return Err(CoreError::Config(
                "Owner native window is not available".into(),
            ));
        }
        // SAFETY: Tauri returns its live NSWindow pointer. The window is owned by
        // the AppHandle and this reference is used only on the AppKit main thread.
        let parent: &NSWindow = unsafe { &*raw_parent.cast::<NSWindow>() };
        let panel = NSOpenPanel::openPanel(mtm);
        panel.setCanChooseFiles(!directory);
        panel.setCanChooseDirectories(directory);
        panel.setAllowsMultipleSelection(!directory);
        panel.setResolvesAliases(true);
        panel.setTitle(Some(&NSString::from_str(if directory {
            "Select output folder"
        } else {
            "Select media files"
        })));

        let callback_panel = panel.clone();
        let sender = Mutex::new(Some(sender));
        let completion = RcBlock::new(move |response| {
            let result = if response == NSModalResponseOK {
                selected_paths(&callback_panel)
            } else if response == NSModalResponseCancel {
                Ok(Vec::new())
            } else {
                Err(CoreError::Config(format!(
                    "native media picker failed with AppKit response {response}"
                )))
            };
            if let Some(sender) = sender.lock().unwrap().take() {
                let _ = sender.send(result);
            }
        });
        // Returns immediately. AppKit owns the sheet presentation and invokes the
        // copied completion block after selection; no runModal/process wait exists.
        panel.beginSheetModalForWindow_completionHandler(parent, &completion);
        Ok(())
    }

    fn selected_paths(panel: &NSOpenPanel) -> CoreResult<Vec<PathBuf>> {
        let urls = panel.URLs();
        let mut paths = Vec::with_capacity(urls.count());
        for index in 0..urls.count() {
            let url = urls.objectAtIndex(index);
            let path = url.path().ok_or_else(|| {
                CoreError::Config("selected file URL has no filesystem path".into())
            })?;
            paths.push(PathBuf::from(path.to_string()));
        }
        Ok(paths)
    }

    fn internal(error: impl std::fmt::Display) -> CoreError {
        CoreError::Internal(anyhow::anyhow!(error.to_string()))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use objc2_foundation::NSURL;

        #[test]
        fn file_urls_preserve_unicode_spaces_and_multiple_selection_order() {
            let expected = vec![
                PathBuf::from("/tmp/媒体 文件 01.png"),
                PathBuf::from("/tmp/第二个 文件.mp4"),
            ];
            let urls = expected
                .iter()
                .map(|path| NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy())))
                .collect::<Vec<_>>();
            let actual = urls
                .iter()
                .map(|url| PathBuf::from(url.path().unwrap().to_string()))
                .collect::<Vec<_>>();
            assert_eq!(actual, expected);
        }
    }
}

#[cfg(target_os = "windows")]
#[path = "native_media_windows.rs"]
mod platform;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use super::*;

    pub(super) async fn pick(
        _app: &AppHandle,
        _host: &Arc<DesktopHost>,
        _directory: bool,
        _owner_label: &str,
    ) -> CoreResult<Vec<PathBuf>> {
        Err(CoreError::Config(
            "native file selection is not supported on this platform; use drag and drop".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_and_selection_payloads_match_the_media_ipc_contract() {
        assert_eq!(selection_payload(false, &[]), serde_json::json!([]));
        assert_eq!(selection_payload(true, &[]), Value::Null);
        let paths = vec![
            PathBuf::from("/tmp/媒体 文件 01.png"),
            PathBuf::from("/tmp/第二个 文件.mp4"),
        ];
        assert_eq!(selection_payload(false, &paths), serde_json::json!(paths));
        assert_eq!(selection_payload(true, &paths), serde_json::json!(paths[0]));
    }

    #[test]
    fn module_disable_is_rejected_before_any_native_dialog_work() {
        let dir = tempfile::tempdir().unwrap();
        let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
        host.kernel()
            .set_module_enabled("media_proc", false)
            .unwrap();
        assert!(ensure_media_enabled(&host).is_err());
        host.kernel()
            .set_module_enabled("media_proc", true)
            .unwrap();
        assert!(ensure_media_enabled(&host).is_ok());
    }
}
