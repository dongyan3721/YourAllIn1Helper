//! Windows 媒体选择器：由本程序持有的原生 COM 对话框，不依赖 PowerShell/WinForms。
use super::*;
use std::ffi::c_void;
use tauri::Manager;
use tokio::sync::oneshot;
use windows::core::{HRESULT, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FileOpenDialog, IFileOpenDialog, IShellItem, FILEOPENDIALOGOPTIONS, FOS_ALLOWMULTISELECT,
    FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS,
    SIGDN_FILESYSPATH,
};

pub(super) async fn pick(
    app: &AppHandle,
    _host: &Arc<DesktopHost>,
    directory: bool,
    owner_label: &str,
) -> CoreResult<Vec<PathBuf>> {
    let window = app
        .get_webview_window(owner_label)
        .ok_or_else(|| CoreError::Config("Owner window is not available".into()))?;
    let owner = window.hwnd().map_err(internal)?.0 as isize;
    let (sender, receiver) = oneshot::channel();
    // 新建专用 STA，避免复用已被其他服务初始化成 MTA 的阻塞线程。
    std::thread::Builder::new()
        .name("media-file-picker".into())
        .spawn(move || {
            let _ = sender.send(run_picker(owner, directory));
        })?;
    receiver
        .await
        .map_err(|error| CoreError::Internal(anyhow::anyhow!(error.to_string())))?
}

struct ComApartment;
impl ComApartment {
    fn init() -> CoreResult<Self> {
        // SAFETY: 仅在本次创建的 STA 工作线程上调用，并与 Drop 配对。
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(internal)?;
        }
        Ok(Self)
    }
}
impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn configure(dialog: &IFileOpenDialog, directory: bool) -> CoreResult<()> {
    let title = wide(if directory {
        "选择输出目录"
    } else {
        "选择媒体文件"
    });
    let media_name = wide("媒体文件");
    let media_spec = wide("*.png;*.jpg;*.jpeg;*.webp;*.bmp;*.ico;*.mp3;*.wav;*.flac;*.aac;*.m4a;*.ogg;*.mp4;*.mkv;*.mov;*.webm");
    let all_name = wide("所有文件");
    let all_spec = wide("*.*");
    let filters = [
        COMDLG_FILTERSPEC {
            pszName: PCWSTR(media_name.as_ptr()),
            pszSpec: PCWSTR(media_spec.as_ptr()),
        },
        COMDLG_FILTERSPEC {
            pszName: PCWSTR(all_name.as_ptr()),
            pszSpec: PCWSTR(all_spec.as_ptr()),
        },
    ];
    // SAFETY: 字符串在所有调用期间存活；COM 会复制标题与过滤器。
    unsafe {
        dialog.SetTitle(PCWSTR(title.as_ptr())).map_err(internal)?;
        let common = FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST | FOS_NOCHANGEDIR;
        let mode = if directory {
            FOS_PICKFOLDERS
        } else {
            FOS_ALLOWMULTISELECT | FOS_FILEMUSTEXIST
        };
        let original = dialog.GetOptions().map_err(internal)?.0;
        let base = original & !(FOS_ALLOWMULTISELECT.0 | FOS_FILEMUSTEXIST.0 | FOS_PICKFOLDERS.0);
        dialog
            .SetOptions(FILEOPENDIALOGOPTIONS(base) | common | mode)
            .map_err(internal)?;
        if !directory {
            dialog.SetFileTypes(&filters).map_err(internal)?;
        }
    }
    Ok(())
}

fn confirmed(result: windows::core::Result<()>) -> CoreResult<bool> {
    const CANCELLED: HRESULT = HRESULT(0x8007_04C7u32 as i32);
    const ABORTED: HRESULT = HRESULT(0x8000_4004u32 as i32);
    match result {
        Ok(()) => Ok(true),
        Err(error) if error.code() == CANCELLED || error.code() == ABORTED => Ok(false),
        Err(error) => Err(internal(error)),
    }
}

fn item_path(item: &IShellItem) -> CoreResult<PathBuf> {
    // SAFETY: 只请求文件系统路径；Shell 返回的内存始终在转换后释放，包括转换失败。
    unsafe {
        let raw = item.GetDisplayName(SIGDN_FILESYSPATH).map_err(internal)?;
        if raw.is_null() {
            return Err(CoreError::Config("系统选择器返回空路径".into()));
        }
        let result = raw.to_string().map(PathBuf::from).map_err(internal);
        CoTaskMemFree(Some(raw.0 as *const c_void));
        result
    }
}

fn run_picker(owner: isize, directory: bool) -> CoreResult<Vec<PathBuf>> {
    if owner == 0 {
        return Err(CoreError::Config(
            "Owner native window is not available".into(),
        ));
    }
    let _com = ComApartment::init()?;
    // SAFETY: COM 对象的全部操作与析构均在当前 STA 内完成。
    let dialog: IFileOpenDialog =
        unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).map_err(internal)? };
    configure(&dialog, directory)?;
    if !confirmed(unsafe { dialog.Show(Some(HWND(owner as *mut c_void))) })? {
        return Ok(vec![]);
    }
    if directory {
        return Ok(vec![item_path(&unsafe {
            dialog.GetResult().map_err(internal)?
        })?]);
    }
    let items = unsafe { dialog.GetResults().map_err(internal)? };
    let count = unsafe { items.GetCount().map_err(internal)? };
    (0..count)
        .map(|i| item_path(&unsafe { items.GetItemAt(i).map_err(internal)? }))
        .collect()
}

fn internal(error: impl std::fmt::Display) -> CoreError {
    CoreError::Internal(anyhow::anyhow!("Windows 媒体选择器：{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_is_empty_but_other_com_errors_are_reported() {
        for code in [0x8007_04C7u32, 0x8000_4004] {
            assert!(!confirmed(Err(windows::core::Error::from_hresult(HRESULT(
                code as i32
            ))))
            .unwrap());
        }
        assert!(confirmed(Ok(())).unwrap());
        assert!(confirmed(Err(windows::core::Error::from_hresult(HRESULT(
            0x8007_0005u32 as i32
        ))))
        .is_err());
    }
    #[test]
    fn real_com_dialog_modes_keep_files_and_folders_separate_without_showing_ui() {
        let _com = ComApartment::init().unwrap();
        for directory in [false, true] {
            let dialog: IFileOpenDialog =
                unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).unwrap() };
            configure(&dialog, directory).unwrap();
            let options = unsafe { dialog.GetOptions().unwrap() };
            assert_ne!(options.0 & FOS_FORCEFILESYSTEM.0, 0);
            assert_ne!(options.0 & FOS_PATHMUSTEXIST.0, 0);
            assert_eq!(options.0 & FOS_PICKFOLDERS.0 != 0, directory);
            assert_eq!(options.0 & FOS_ALLOWMULTISELECT.0 != 0, !directory);
            // Windows may restore FILEMUSTEXIST for folder pickers; the selected
            // item must exist in both modes. Only file mode requires this bit.
            if !directory {
                assert_ne!(options.0 & FOS_FILEMUSTEXIST.0, 0);
            }
        }
    }
}
