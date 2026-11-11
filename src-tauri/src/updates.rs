//! Signed, per-user Windows updates. Downloads never interrupt an active session.
use parking_lot::Mutex;
use std::{path::Path, sync::Arc, time::Duration};
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Default)]
pub(crate) struct UpdateState {
    pending: Mutex<Option<(Update, Vec<u8>)>>,
}

fn same_install_directory(executable: &Path, install_directory: &Path) -> bool {
    executable.parent().is_some_and(|parent| {
        let normalize = |p: &Path| {
            p.to_string_lossy()
                .trim_end_matches(['\\', '/'])
                .replace('/', "\\")
                .to_lowercase()
        };
        normalize(parent) == normalize(install_directory)
    })
}

/// Debug/QA and portable copies must never accidentally update an installed copy.
fn is_installed_release() -> bool {
    if cfg!(debug_assertions) || std::env::var_os("ALL_IN_ONE_GUI_TEST_DATA_DIR").is_some() {
        return false;
    }
    let Ok(key) = windows_registry::CURRENT_USER.open(r"Software\dongyan3721\All-in-One Toolbox")
    else {
        return false;
    };
    let Ok(directory) = key.get_string("InstallDir") else {
        return false;
    };
    std::env::current_exe().is_ok_and(|exe| same_install_directory(&exe, Path::new(&directory)))
}

pub(crate) fn start(app: &AppHandle) {
    if !is_installed_release() {
        return;
    }
    tracing::info!("Installed Windows release: background signed updates enabled");
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(15)).await;
        loop {
            if crate::tauri_app::is_exiting(&app) {
                return;
            }
            let state = app.state::<Arc<UpdateState>>();
            if state.pending.lock().is_none() {
                if let Err(error) = check_and_download(&app, &state).await {
                    tracing::warn!(%error, "Background update check/download failed; continuing current version");
                }
            }
            tokio::time::sleep(Duration::from_secs(6 * 60 * 60)).await;
        }
    });
}

async fn check_and_download(
    app: &AppHandle,
    state: &UpdateState,
) -> tauri_plugin_updater::Result<()> {
    let updater = app
        .updater_builder()
        .timeout(Duration::from_secs(120))
        .restart_after_install(false)
        .build()?;
    if let Some(update) = updater.check().await? {
        let bytes = update.download(|_, _| {}, || {}).await?;
        // download() verifies the package signature. install() verifies it again.
        if !crate::tauri_app::is_exiting(app) {
            tracing::info!(version = %update.version, "Signed update downloaded; installation deferred until clean exit");
            *state.pending.lock() = Some((update, bytes));
        }
    } else {
        tracing::info!("Background update check completed: current version is up to date");
    }
    Ok(())
}

/// Called only after the existing exit transaction has flushed notes and shut down modules.
/// On Windows the plugin starts msiexec /quiet and exits this process on success.
pub(crate) fn install_after_shutdown(app: &AppHandle) {
    let Some(state) = app.try_state::<Arc<UpdateState>>() else {
        return;
    };
    let pending = state.pending.lock().take();
    if let Some((update, bytes)) = pending {
        if let Err(error) = update.install(bytes) {
            tracing::error!(%error, "Update installation did not start; the installed version is retained");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installed_path_match_rejects_portable_and_sibling_copies() {
        assert!(same_install_directory(
            Path::new(
                r"C:\Users\QA\AppData\Local\Programs\All-in-One Toolbox\all-in-one-desktop.exe"
            ),
            Path::new(r"c:\users\qa\appdata\local\programs\All-in-One Toolbox\")
        ));
        assert!(!same_install_directory(
            Path::new(r"D:\Downloads\all-in-one-desktop.exe"),
            Path::new(r"C:\Users\QA\AppData\Local\Programs\All-in-One Toolbox")
        ));
        assert!(!same_install_directory(
            Path::new(r"C:\Apps\All-in-One Toolbox-old\all-in-one-desktop.exe"),
            Path::new(r"C:\Apps\All-in-One Toolbox")
        ));
    }
}
