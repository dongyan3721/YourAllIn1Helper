//! 截图「另存为」原生保存对话框与一次性保存路径授权。
//!
//! 设计约束（`docs/WINDOWS_GUI_FIXES_COORDINATION_2026-11-08.md`）：
//! - 对话框保持异步：macOS 用 AppKit sheet + completion handler；Windows 在阻塞线程池上
//!   运行 `IFileSaveDialog` 自有模态循环，二者都不阻塞 IPC/UI 线程。
//! - 不做任何 shell 拼接：用户路径只以 `PathBuf`/UTF-16 传给系统 API，绝不进入命令行。
//! - 覆盖确认由系统对话框完成（`FOS_OVERWRITEPROMPT` / NSSavePanel 默认确认）；
//!   写入侧只接受本对话框登记过的一次性路径令牌（[`SavePathRegistry`]），
//!   防止 HTTP/WebView 调用方绕过覆盖确认写任意路径。
//! - 取消返回 `path: null`，不产生任何写入。
use crate::{DesktopHost, IpcResponse};
use app_core::{CoreError, CoreResult};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tauri::AppHandle;

/// 保存产物格式；决定扩展名、过滤器与路径令牌的类型匹配。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveKind {
    Png,
    Gif,
}

impl SaveKind {
    pub fn extension(self) -> &'static str {
        match self {
            SaveKind::Png => "png",
            SaveKind::Gif => "gif",
        }
    }

    fn from_str(value: &str) -> CoreResult<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "png" => Ok(SaveKind::Png),
            "gif" => Ok(SaveKind::Gif),
            other => Err(CoreError::Config(format!("未知保存格式: {}", other))),
        }
    }
}

/// 一次保存对话框的展示参数（标题/过滤器文案由前端按当前语言提供）。
#[derive(Debug, Clone)]
pub struct SaveDialogSpec {
    pub title: String,
    pub default_file_name: String,
    pub kind: SaveKind,
    pub filter_label: String,
    pub owner_label: String,
}

/// 一次性「另存为」路径授权表：只有系统保存对话框确认过的路径才允许覆盖写入。
#[derive(Default)]
pub struct SavePathRegistry {
    recent: Mutex<VecDeque<(PathBuf, SaveKind)>>,
}

/// 令牌容量：同时存在多个保存流程时保留最近若干个选择，超出即作废最旧的。
const MAX_RECENT_SELECTIONS: usize = 8;

impl SavePathRegistry {
    /// 登记保存对话框的确认结果（取消不登记）。
    pub fn register(&self, path: PathBuf, kind: SaveKind) {
        let mut recent = self.recent.lock().unwrap();
        recent.retain(|(existing, _)| existing != &path);
        recent.push_back((path, kind));
        while recent.len() > MAX_RECENT_SELECTIONS {
            recent.pop_front();
        }
    }

    /// 消费一次授权：路径与格式都必须命中且未被用过；失败即拒绝覆盖写入。
    pub fn consume(&self, path: &Path, kind: SaveKind) -> CoreResult<()> {
        let mut recent = self.recent.lock().unwrap();
        if let Some(index) = recent
            .iter()
            .position(|(existing, saved_kind)| existing == path && *saved_kind == kind)
        {
            recent.remove(index);
            return Ok(());
        }
        Err(CoreError::Config(
            "保存路径必须来自系统保存对话框的「另存为」选择".into(),
        ))
    }
}

/// 由前端提供的默认文件名净化为安全的叶子文件名：
/// 去掉目录分隔符与 Windows 非法字符（保留中文与空格），并保证带目标扩展名。
pub fn sanitize_file_name(raw: &str, kind: SaveKind) -> String {
    let leaf = raw
        .rsplit(['/', '\\'])
        .find(|part| !part.trim().is_empty())
        .unwrap_or("");
    let cleaned: String = leaf
        .chars()
        .map(|c| {
            if c == ':'
                || c == '*'
                || c == '?'
                || c == '"'
                || c == '<'
                || c == '>'
                || c == '|'
                || c.is_control()
            {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim();
    let fallback = format!("screenshot.{}", kind.extension());
    if cleaned.is_empty() {
        return fallback;
    }
    let path = mod_screenshot::export::resolve_save_path(Path::new(cleaned), kind.extension());
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or(fallback)
}

/// 解析 `screenshot_pick_save_path` 的 JSON 参数。
pub(crate) fn parse_spec(args: &Value) -> CoreResult<SaveDialogSpec> {
    let kind = SaveKind::from_str(
        args.get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| CoreError::Config("缺少参数 kind".into()))?,
    )?;
    let filter_label = args
        .get("filter_label")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(match kind {
            SaveKind::Png => "PNG image",
            SaveKind::Gif => "GIF animation",
        })
        .to_string();
    Ok(SaveDialogSpec {
        title: args
            .get("title")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("保存截图")
            .to_string(),
        default_file_name: sanitize_file_name(
            args.get("default_name")
                .and_then(Value::as_str)
                .unwrap_or(""),
            kind,
        ),
        kind,
        filter_label,
        owner_label: String::new(),
    })
}

/// 前端指定的对话框属主窗口必须存在，否则退回主面板（与媒体选择器一致）。
fn resolve_owner_label(app: &AppHandle, requested: Option<&str>) -> String {
    use tauri::Manager;
    if let Some(label) = requested.filter(|value| !value.is_empty()) {
        if app.get_webview_window(label).is_some() {
            return label.to_string();
        }
    }
    "dashboard".to_string()
}

fn ensure_screenshot_enabled(host: &DesktopHost) -> CoreResult<()> {
    host.kernel()
        .registry()
        .with_enabled("screenshot", || Ok(()))
}

/// 同一时刻只允许一个系统保存对话框，避免多窗口并发呼出叠加。
/// 用原子标志而非互斥锁守卫：守卫需跨 `.await` 持有，必须保持 `Send`。
static SAVE_DIALOG_ACTIVE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

struct SingleDialogGuard;

impl SingleDialogGuard {
    fn acquire() -> CoreResult<Self> {
        if SAVE_DIALOG_ACTIVE
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_err()
        {
            return Err(CoreError::Config(
                "保存对话框已打开，请先完成或取消当前选择".into(),
            ));
        }
        Ok(Self)
    }
}

impl Drop for SingleDialogGuard {
    fn drop(&mut self) {
        SAVE_DIALOG_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// `screenshot_pick_save_path`：展示系统保存对话框；取消返回 `{"path": null}`，
/// 确认返回完整路径并登记一次性授权。异步执行，不阻塞 UI。
pub(crate) async fn pick_screenshot_save_path(
    app: &AppHandle,
    host: &Arc<DesktopHost>,
    args: Value,
) -> IpcResponse {
    let result = pick_screenshot_save_path_result(app, host, args).await;
    match result {
        Ok(path) => IpcResponse::success(json!({
            "path": path.map(|p| p.to_string_lossy().into_owned()),
        })),
        Err(error) => IpcResponse::err(error.to_string()),
    }
}

async fn pick_screenshot_save_path_result(
    app: &AppHandle,
    host: &Arc<DesktopHost>,
    args: Value,
) -> CoreResult<Option<PathBuf>> {
    ensure_screenshot_enabled(host)?;
    let mut spec = parse_spec(&args)?;
    spec.owner_label = resolve_owner_label(app, args.get("owner_label").and_then(Value::as_str));
    let _active_dialog = SingleDialogGuard::acquire()?;
    let chosen = platform::show_save_dialog(app, &spec).await?;
    if let Some(path) = chosen.as_ref() {
        host.save_paths().register(path.clone(), spec.kind);
    }
    Ok(chosen)
}

#[cfg(target_os = "windows")]
mod platform {
    use super::*;
    use std::ffi::c_void;
    use tauri::Manager;
    use windows::core::{HRESULT, PCWSTR, PWSTR};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::{
        FileSaveDialog, IFileSaveDialog, IShellItem, FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR,
        FOS_OVERWRITEPROMPT, FOS_STRICTFILETYPES, SIGDN_FILESYSPATH,
    };

    /// `HRESULT_FROM_WIN32(ERROR_CANCELLED)`；`E_ABORT` 也按取消处理。
    const ERROR_CANCELLED_HRESULT: HRESULT = HRESULT(0x8007_04C7u32 as i32);
    const E_ABORT: HRESULT = HRESULT(0x8000_4004u32 as i32);

    fn is_cancel(code: HRESULT) -> bool {
        code == ERROR_CANCELLED_HRESULT || code == E_ABORT
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// COM 单线程公寓生命周期守卫：进入时 `CoInitializeEx`，退出时 `CoUninitialize`。
    struct ComApartment;

    impl ComApartment {
        fn init() -> CoreResult<Self> {
            // SAFETY: 阻塞线程池线程上以 STA 初始化 COM；配对的 CoUninitialize 在 Drop。
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
                .ok()
                .map_err(|error| {
                    CoreError::Internal(anyhow::anyhow!("COM 初始化失败：{}", error.message()))
                })?;
            Ok(Self)
        }
    }

    impl Drop for ComApartment {
        fn drop(&mut self) {
            // SAFETY: 与 init 配对。
            unsafe { CoUninitialize() };
        }
    }

    pub(super) async fn show_save_dialog(
        app: &AppHandle,
        spec: &SaveDialogSpec,
    ) -> CoreResult<Option<PathBuf>> {
        let owner = owner_hwnd(app, &spec.owner_label)?;
        let spec = spec.clone();
        tauri::async_runtime::spawn_blocking(move || run_save_dialog(owner, &spec))
            .await
            .map_err(|error| CoreError::Internal(anyhow::anyhow!(error.to_string())))?
    }

    fn owner_hwnd(app: &AppHandle, owner_label: &str) -> CoreResult<isize> {
        let window = app
            .get_webview_window(owner_label)
            .ok_or_else(|| CoreError::Config(format!("窗口 {} 不可用", owner_label)))?;
        // Tauri 返回 Win32 HWND 包装类型，取内部指针再转句柄整数；不截断 Win64 高位。
        let raw = window
            .hwnd()
            .map_err(|error| CoreError::Internal(anyhow::anyhow!(error.to_string())))?;
        Ok(raw.0 as isize)
    }

    fn run_save_dialog(owner: isize, spec: &SaveDialogSpec) -> CoreResult<Option<PathBuf>> {
        let _com = ComApartment::init()?;
        // SAFETY: COM 已在本线程以 STA 初始化；对话框对象在本函数内全程持有。
        let dialog: IFileSaveDialog = unsafe {
            CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER).map_err(|error| {
                CoreError::Internal(anyhow::anyhow!(
                    "创建系统保存对话框失败：{}",
                    error.message()
                ))
            })?
        };
        let name_w = wide(&spec.filter_label);
        let ext_w = wide(&format!("*.{}", spec.kind.extension()));
        let filters = [COMDLG_FILTERSPEC {
            pszName: PCWSTR(name_w.as_ptr()),
            pszSpec: PCWSTR(ext_w.as_ptr()),
        }];
        let default_ext = wide(spec.kind.extension());
        let file_name = wide(&spec.default_file_name);
        let title = wide(&spec.title);
        // SAFETY: 传入的宽字符串在调用期间保持存活；COM 会复制这些缓冲区。
        unsafe {
            dialog
                .SetTitle(PCWSTR(title.as_ptr()))
                .map_err(|error| internal_error("设置保存标题", error))?;
            dialog
                .SetFileTypes(&filters)
                .map_err(|error| internal_error("设置文件类型", error))?;
            dialog
                .SetDefaultExtension(PCWSTR(default_ext.as_ptr()))
                .map_err(|error| internal_error("设置默认扩展名", error))?;
            dialog
                .SetFileName(PCWSTR(file_name.as_ptr()))
                .map_err(|error| internal_error("设置默认文件名", error))?;
            let options = dialog
                .GetOptions()
                .map_err(|error| internal_error("读取对话框选项", error))?;
            dialog
                .SetOptions(
                    options
                        | FOS_FORCEFILESYSTEM
                        | FOS_NOCHANGEDIR
                        | FOS_OVERWRITEPROMPT
                        | FOS_STRICTFILETYPES,
                )
                .map_err(|error| internal_error("设置对话框选项", error))?;
        }
        let hwnd = HWND(owner as *mut c_void);
        // SAFETY: 属主窗口句柄来自 Tauri 实时窗口；Show 在本线程跑模态循环，
        // IPC/UI 线程不受阻塞。覆盖确认由 FOS_OVERWRITEPROMPT 在对话框内完成。
        match unsafe { dialog.Show(Some(hwnd)) } {
            Ok(()) => {}
            Err(error) if is_cancel(error.code()) => return Ok(None),
            Err(error) => return Err(internal_error("显示保存对话框", error)),
        }
        // SAFETY: Show 成功后 GetResult 返回选中项；GetDisplayName 分配的缓冲区用
        // CoTaskMemFree 释放，路径以 UTF-16 原样转 PathBuf，不经 shell。
        unsafe {
            let item: IShellItem = dialog
                .GetResult()
                .map_err(|error| internal_error("读取保存选择", error))?;
            let raw: PWSTR = item
                .GetDisplayName(SIGDN_FILESYSPATH)
                .map_err(|error| internal_error("读取保存路径", error))?;
            if raw.is_null() {
                return Err(CoreError::Internal(anyhow::anyhow!("保存路径为空")));
            }
            let path = raw.to_string().map_err(|error| {
                CoreError::Internal(anyhow::anyhow!("转换保存路径失败：{}", error))
            })?;
            CoTaskMemFree(Some(raw.0 as *const c_void));
            Ok(Some(PathBuf::from(path)))
        }
    }

    fn internal_error(action: &str, error: windows::core::Error) -> CoreError {
        CoreError::Internal(anyhow::anyhow!("{}失败：{}", action, error.message()))
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use block2::RcBlock;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSModalResponseCancel, NSModalResponseOK, NSSavePanel, NSWindow};
    use objc2_foundation::NSString;
    use tauri::Manager;
    use tokio::sync::oneshot;

    pub(super) async fn show_save_dialog(
        app: &AppHandle,
        spec: &SaveDialogSpec,
    ) -> CoreResult<Option<PathBuf>> {
        let (sender, receiver) = oneshot::channel();
        let app = app.clone();
        let spec = spec.clone();
        app.clone()
            .run_on_main_thread(move || {
                if let Err(error) = present_sheet(&app, &spec, sender) {
                    tracing::error!(%error, "Failed to present native save panel");
                }
            })
            .map_err(|error| CoreError::Internal(anyhow::anyhow!(error.to_string())))?;
        receiver.await.map_err(|_| {
            CoreError::Config("native save panel closed without returning a result".into())
        })?
    }

    fn present_sheet(
        app: &AppHandle,
        spec: &SaveDialogSpec,
        sender: oneshot::Sender<CoreResult<Option<PathBuf>>>,
    ) -> CoreResult<()> {
        let mtm = MainThreadMarker::new().ok_or_else(|| {
            CoreError::Config("native save panel must run on the AppKit main thread".into())
        })?;
        let window = app
            .get_webview_window(&spec.owner_label)
            .ok_or_else(|| CoreError::Config("owner window is not available".into()))?;
        window.show().map_err(internal)?;
        window.set_focus().map_err(internal)?;
        let raw_parent = window.ns_window().map_err(internal)?;
        if raw_parent.is_null() {
            return Err(CoreError::Config(
                "owner native window is not available".into(),
            ));
        }
        // SAFETY: Tauri 返回其持有的 NSWindow 指针，仅在 AppKit 主线程使用。
        let parent: &NSWindow = unsafe { &*raw_parent.cast::<NSWindow>() };
        let panel = NSSavePanel::savePanel(mtm);
        panel.setTitle(Some(&NSString::from_str(&spec.title)));
        panel.setNameFieldStringValue(&NSString::from_str(&spec.default_file_name));
        panel.setCanCreateDirectories(true);
        panel.setExtensionHidden(false);

        let callback_panel = panel.clone();
        let sender = std::sync::Mutex::new(Some(sender));
        let completion = RcBlock::new(move |response| {
            let result = if response == NSModalResponseOK {
                Ok(selected_path(&callback_panel))
            } else if response == NSModalResponseCancel {
                Ok(None)
            } else {
                Err(CoreError::Config(format!(
                    "native save panel failed with AppKit response {response}"
                )))
            };
            if let Some(sender) = sender.lock().unwrap().take() {
                let _ = sender.send(result);
            }
        });
        // 返回立即；AppKit 持有 sheet 并在用户选择后回调，无 runModal 等待。
        panel.beginSheetModalForWindow_completionHandler(parent, &completion);
        Ok(())
    }

    fn selected_path(panel: &NSSavePanel) -> Option<PathBuf> {
        panel
            .URL()
            .and_then(|url| url.path())
            .map(|path| PathBuf::from(path.to_string()))
    }

    fn internal(error: impl std::fmt::Display) -> CoreError {
        CoreError::Internal(anyhow::anyhow!(error.to_string()))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use super::*;

    pub(super) async fn show_save_dialog(
        _app: &AppHandle,
        _spec: &SaveDialogSpec,
    ) -> CoreResult<Option<PathBuf>> {
        Err(CoreError::Config(
            "native save dialog is not supported on this platform".into(),
        ))
    }
}

#[cfg(test)]
#[path = "../tests/unit/native_save_dialog.rs"]
mod tests;
