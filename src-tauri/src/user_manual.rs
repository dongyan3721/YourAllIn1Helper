//! Open the bundled manual as an ordinary local file in the user's editor.
use app_core::{CoreError, CoreResult};
use std::path::{Path, PathBuf};
use tauri::AppHandle;

const MANUAL: &str = include_str!("../../docs/USER_GUIDE.md");

fn materialize(data_dir: &Path) -> CoreResult<PathBuf> {
    let directory = data_dir.join("docs");
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("USER_GUIDE.md");
    if std::fs::read(&path).ok().as_deref() != Some(MANUAL.as_bytes()) {
        std::fs::write(&path, MANUAL)?;
    }
    Ok(path)
}

pub(crate) fn open(app: &AppHandle) -> CoreResult<PathBuf> {
    let directory = crate::tauri_app::manual_data_dir(app)?;
    let path = materialize(&directory)?;
    open_path(&path)?;
    Ok(path)
}

#[cfg(target_os = "windows")]
fn open_path(path: &Path) -> CoreResult<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::{w, PCWSTR},
        Win32::{
            Foundation::HWND,
            UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        },
    };
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let result = unsafe {
        ShellExecuteW(
            Some(HWND::default()),
            w!("open"),
            PCWSTR(wide.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    }
    .0 as isize;
    if result > 32 {
        return Ok(());
    }
    if result == 31 {
        // No default Markdown association: use the installed text editor.
        std::process::Command::new("notepad.exe")
            .arg(path)
            .spawn()?;
        return Ok(());
    }
    Err(CoreError::Config(format!(
        "Cannot open the local user guide (Windows error {result})"
    )))
}

#[cfg(not(target_os = "windows"))]
fn open_path(path: &Path) -> CoreResult<()> {
    let command = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(command).arg(path).spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manual_is_one_fixed_local_file_and_can_be_refreshed() {
        let dir = tempfile::tempdir().unwrap();
        let path = materialize(dir.path()).unwrap();
        assert_eq!(path, dir.path().join("docs/USER_GUIDE.md"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), MANUAL);
        std::fs::write(&path, "outdated bundled manual").unwrap();
        assert_eq!(materialize(dir.path()).unwrap(), path);
        assert_eq!(std::fs::read_to_string(path).unwrap(), MANUAL);
    }
}
