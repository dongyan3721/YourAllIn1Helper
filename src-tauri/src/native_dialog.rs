//! Host-owned native media pickers. Feature modules never depend on UI frameworks.
//!
//! macOS presents an AppKit sheet on the Dashboard's real NSWindow. The sheet uses a
//! completion handler, so neither the AppKit event loop nor the async IPC task is blocked.
//! Windows keeps its existing WinForms implementation off the Tauri event thread.

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
) -> IpcResponse {
    let result = pick_media_result(app, host, directory).await;
    match result {
        Ok(paths) => IpcResponse::success(selection_payload(directory, &paths)),
        Err(error) => IpcResponse::err(error.to_string()),
    }
}

async fn pick_media_result(
    app: &AppHandle,
    host: &Arc<DesktopHost>,
    directory: bool,
) -> CoreResult<Vec<PathBuf>> {
    ensure_media_enabled(host)?;
    let paths = platform::pick(app, host, directory).await?;
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
    ) -> CoreResult<Vec<PathBuf>> {
        let (sender, receiver) = oneshot::channel();
        let app = app.clone();
        app.clone()
            .run_on_main_thread(move || {
                if let Err(error) = present_dashboard_sheet(&app, directory, sender) {
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
        sender: oneshot::Sender<CoreResult<Vec<PathBuf>>>,
    ) -> CoreResult<()> {
        let mtm = MainThreadMarker::new().ok_or_else(|| {
            CoreError::Config("native media picker must run on the AppKit main thread".into())
        })?;
        let window = app
            .get_webview_window("dashboard")
            .ok_or_else(|| CoreError::Config("Dashboard window is not available".into()))?;
        window.show().map_err(internal)?;
        window.set_focus().map_err(internal)?;
        let raw_parent = window.ns_window().map_err(internal)?;
        if raw_parent.is_null() {
            return Err(CoreError::Config(
                "Dashboard native window is not available".into(),
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
mod platform {
    use super::*;
    use app_core::ExternalToolKind;
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    use tauri::Manager;

    pub(super) async fn pick(
        app: &AppHandle,
        host: &Arc<DesktopHost>,
        directory: bool,
    ) -> CoreResult<Vec<PathBuf>> {
        let owner = dashboard_hwnd(app)?;
        let host = host.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let binary = host
                .kernel()
                .context()
                .env_probe
                .require_executable(ExternalToolKind::PowerShell)?;
            let script = if directory {
                format!(
                    "[Console]::OutputEncoding=[Text.Encoding]::UTF8; Add-Type -AssemblyName System.Windows.Forms; Add-Type 'using System; using System.Windows.Forms; public class DialogOwner : IWin32Window {{ public IntPtr Handle {{get;}} public DialogOwner(IntPtr h) {{Handle=h;}} }}'; $d=New-Object System.Windows.Forms.FolderBrowserDialog; $o=New-Object DialogOwner([IntPtr]{owner}); if($d.ShowDialog($o)-eq 'OK'){{$d.SelectedPath}}"
                )
            } else {
                format!(
                    "[Console]::OutputEncoding=[Text.Encoding]::UTF8; Add-Type -AssemblyName System.Windows.Forms; Add-Type 'using System; using System.Windows.Forms; public class DialogOwner : IWin32Window {{ public IntPtr Handle {{get;}} public DialogOwner(IntPtr h) {{Handle=h;}} }}'; $d=New-Object System.Windows.Forms.OpenFileDialog; $d.Multiselect=$true; $d.Filter='Media files|*.png;*.jpg;*.jpeg;*.webp;*.bmp;*.ico;*.mp3;*.wav;*.flac;*.aac;*.m4a;*.ogg;*.mp4;*.mkv;*.mov;*.webm|All files|*.*'; $o=New-Object DialogOwner([IntPtr]{owner}); if($d.ShowDialog($o)-eq 'OK'){{$d.FileNames}}"
                )
            };
            let mut command = Command::new(binary);
            command
                .args(["-NoProfile", "-STA", "-Command", &script])
                .creation_flags(0x08000000);
            let output = command.output()?;
            if !output.status.success() {
                return Err(CoreError::Config(format!(
                    "native file dialog failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                )));
            }
            Ok(parse_windows_paths(&String::from_utf8_lossy(&output.stdout)))
        })
        .await
        .map_err(|error| CoreError::Internal(anyhow::anyhow!(error.to_string())))?
    }

    fn dashboard_hwnd(app: &AppHandle) -> CoreResult<isize> {
        let window = app
            .get_webview_window("dashboard")
            .ok_or_else(|| CoreError::Config("Dashboard window is not available".into()))?;
        Ok(window
            .hwnd()
            .map_err(|error| CoreError::Internal(anyhow::anyhow!(error.to_string())))?
            as isize)
    }

    fn parse_windows_paths(output: &str) -> Vec<PathBuf> {
        output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(PathBuf::from)
            .collect()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn windows_output_preserves_unicode_spaces_and_cancel_is_empty() {
            assert_eq!(
                parse_windows_paths("C:\\媒体 文件\\一 二.png\r\nD:\\第三个 文件.mp4\r\n"),
                vec![
                    PathBuf::from("C:\\媒体 文件\\一 二.png"),
                    PathBuf::from("D:\\第三个 文件.mp4"),
                ]
            );
            assert!(parse_windows_paths("\r\n").is_empty());
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use super::*;

    pub(super) async fn pick(
        _app: &AppHandle,
        _host: &Arc<DesktopHost>,
        _directory: bool,
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
