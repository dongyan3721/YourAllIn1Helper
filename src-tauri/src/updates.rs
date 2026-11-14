//! Signed, per-user desktop updates for Windows and macOS.
//! Downloads never interrupt an active session; installation is deferred to clean exit.
use parking_lot::Mutex;
use std::{path::Path, sync::Arc, time::Duration};
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Default)]
pub(crate) struct UpdateState {
    pending: Mutex<Option<(Update, Vec<u8>)>>,
}

/// Windows 安装目录比较：按 Windows 分隔符解析并归一化大小写。
/// 必须跨平台可测，不能依赖宿主系统的 `Path` 分隔符语义。
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn same_install_directory(executable: &Path, install_directory: &Path) -> bool {
    let executable = executable.to_string_lossy();
    let Some((parent, _file)) = executable.rsplit_once(['\\', '/']) else {
        return false;
    };
    let normalize = |value: &str| {
        value
            .trim_end_matches(['\\', '/'])
            .replace('/', "\\")
            .to_lowercase()
    };
    normalize(parent) == normalize(&install_directory.to_string_lossy())
}

/// Debug/QA and portable copies must never accidentally update an installed copy.
#[cfg(target_os = "windows")]
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

/// macOS 只允许标准 `.app/Contents/MacOS/<bin>` 安装包自动更新；
/// 裸二进制、构建产物与 QA 副本都不参与。
#[cfg(target_os = "macos")]
fn is_installed_release() -> bool {
    if cfg!(debug_assertions) || std::env::var_os("ALL_IN_ONE_GUI_TEST_DATA_DIR").is_some() {
        return false;
    }
    std::env::current_exe()
        .ok()
        .is_some_and(|exe| is_macos_app_bundle_exe(&exe))
}

/// 判断可执行文件是否位于标准 macOS 应用包中。
#[cfg(target_os = "macos")]
fn is_macos_app_bundle_exe(executable: &Path) -> bool {
    use std::ffi::OsStr;
    if executable.file_name() != Some(OsStr::new("all-in-one-desktop")) {
        return false;
    }
    let Some(macos_dir) = executable.parent() else {
        return false;
    };
    if macos_dir.file_name() != Some(OsStr::new("MacOS")) {
        return false;
    }
    let Some(contents_dir) = macos_dir.parent() else {
        return false;
    };
    if contents_dir.file_name() != Some(OsStr::new("Contents")) {
        return false;
    }
    contents_dir
        .parent()
        .is_some_and(|bundle| bundle.extension() == Some(OsStr::new("app")))
}

/// 其他平台暂不开放自动更新，避免把更新包误装到不受支持的运行环境。
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn is_installed_release() -> bool {
    false
}

pub(crate) fn start(app: &AppHandle) {
    if !is_installed_release() {
        return;
    }
    tracing::info!("Installed desktop release: background signed updates enabled");
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
/// Windows starts msiexec /quiet; macOS replaces the signed .app bundle after this process exits.
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

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_bundle_detection_rejects_bare_binary_and_accepts_standard_app() {
        assert!(is_macos_app_bundle_exe(Path::new(
            "/Applications/All-in-One Toolbox.app/Contents/MacOS/all-in-one-desktop"
        )));
        assert!(!is_macos_app_bundle_exe(Path::new(
            "/Users/qa/Downloads/all-in-one-desktop"
        )));
        assert!(!is_macos_app_bundle_exe(Path::new(
            "/Applications/All-in-One Toolbox.app/Contents/MacOS/other-binary"
        )));
    }
}
