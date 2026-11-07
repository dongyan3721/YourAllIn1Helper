//! Windows 剪贴板所有权与动态 GIF 写入（Win32 DataExchange / Memory / WindowsAndMessaging 真实 API）。
//!
//! - 敏感文本写入 `CF_UNICODETEXT`，同时设置 Windows 剪贴板历史与云同步排除标记，
//!   防止系统级剪贴板历史（Win+V）与云剪贴板记录秘密；
//! - 所有权判定使用 `GetClipboardSequenceNumber` 的系统变更序号，不比较文本内容，
//!   用户之后复制相同文本也不会被误清；
//! - 剪贴板所有者是本进程按需创建的消息窗口（`HWND_MESSAGE` 子窗口），
//!   生命周期与单次事务一致（同一线程创建与销毁），不依赖主控台窗口；
//!   文档明确：NULL owner 打开后 `EmptyClipboard` 会使 `SetClipboardData` 失败，故必须提供有效 owner；
//! - `SetClipboardData` 成功后句柄所有权移交系统（不释放）；失败路径一律释放自有句柄。

use crate::{secure::ClipboardWriteToken, HalError, HalResult};
use std::{ptr, thread::sleep, time::Duration};
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{GetLastError, GlobalFree, SetLastError, HANDLE, HGLOBAL, HWND, WIN32_ERROR},
        System::{
            DataExchange::{
                CloseClipboard, EmptyClipboard, GetClipboardSequenceNumber,
                IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW,
                SetClipboardData,
            },
            Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
            Ole::CF_UNICODETEXT,
        },
        UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE,
        },
    },
};

/// 敏感排除标记（微软文档化的注册剪贴板格式名）：
/// 存在该格式时，剪贴板历史与云同步（漫游剪贴板）都不记录本次内容。
pub(super) const EXCLUDE_MONITOR_FORMAT: &str = "Exclude Clipboard Content From Monitor Processing";
/// 细粒度标记：值为 `0`（FALSE）时不进入 Windows 剪贴板历史。
pub(super) const HISTORY_OPT_OUT_FORMAT: &str = "Can Include In Clipboard History";
/// 细粒度标记：值为 `0`（FALSE）时不上传云剪贴板（漫游）。
pub(super) const CLOUD_OPT_OUT_FORMAT: &str = "Can Upload To Cloud Clipboard";

/// 动态 GIF 主格式名（MIME 命名，对应 macOS `com.compuserve.gif`）。
pub(super) const GIF_PRIMARY_FORMAT: &str = "image/gif";
/// 动态 GIF 辅助格式名（短名，对应 macOS `public.gif`，尽力而为）。
pub(super) const GIF_SECONDARY_FORMAT: &str = "GIF";

/// 排除标记的载荷：`BOOL FALSE`（4 字节小端）。内容被系统忽略的标记也统一使用它。
const OPT_OUT_FALSE: &[u8] = &0u32.to_le_bytes();

/// `OpenClipboard` 失败重试次数（剪贴板常被其他进程短暂占用）。
const OPEN_CLIPBOARD_ATTEMPTS: u32 = 10;
/// 每次重试前的等待时间。
const OPEN_CLIPBOARD_BACKOFF: Duration = Duration::from_millis(20);

/// 文本转 UTF-16（含结尾 NUL）；含 NUL 字符的文本会被 Windows 读者截断，直接拒绝。
pub(super) fn utf16_with_nul(text: &str) -> HalResult<Vec<u16>> {
    if text.chars().any(|c| c == '\0') {
        return Err(HalError::NativeCallFailed(
            "敏感文本包含 NUL 字符，写入 Windows 剪贴板会被截断，已拒绝".to_string(),
        ));
    }
    Ok(text.encode_utf16().chain(std::iter::once(0)).collect())
}

/// 把字符串编码为 NUL 结尾的 UTF-16（调用方已保证不含 NUL）。
pub(super) fn to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 剪贴板写入失败的统一错误归类：信息不含剪贴板内容。
pub(super) fn clipboard_error(action: &str, detail: impl std::fmt::Display) -> HalError {
    HalError::NativeCallFailed(format!("Windows 剪贴板{}失败：{}", action, detail))
}

/// 单次剪贴板事务：持有本进程消息窗口（owner）并保持剪贴板打开；
/// 离开作用域自动 `CloseClipboard` 与 `DestroyWindow`（含错误提前返回）。
struct ClipboardSession {
    owner: HWND,
    opened: bool,
}

impl Drop for ClipboardSession {
    fn drop(&mut self) {
        if self.opened {
            // SAFETY: 会话存活期间剪贴板处于打开状态，关闭与打开在同一线程配对。
            let _ = unsafe { CloseClipboard() };
        }
        // SAFETY: 窗口由本线程创建，句柄未被销毁过。
        let _ = unsafe { DestroyWindow(self.owner) };
    }
}

/// 创建剪贴板事务用的所有者窗口：系统预定义 `STATIC` 类 + `HWND_MESSAGE` 父窗口，
/// 不注册窗口类、不显示、不依赖任何既有窗口；同一线程创建并销毁。
pub(super) fn create_owner_window() -> HalResult<HWND> {
    // SAFETY: 纯消息窗口，参数合法（系统类 + 空样式 + HWND_MESSAGE 父窗口）。
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("all-in-one-clipboard-owner"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            None,
            None,
        )
    }
    .map_err(|error| clipboard_error("创建所有者窗口", error))?;
    Ok(hwnd)
}

/// 打开系统剪贴板（owner 为本进程消息窗口），被占用时退避重试；持续失败返回明确错误。
fn open_clipboard() -> HalResult<ClipboardSession> {
    let owner = create_owner_window()?;
    let mut session = ClipboardSession {
        owner,
        opened: false,
    };
    let mut last_error = String::new();
    for attempt in 0..OPEN_CLIPBOARD_ATTEMPTS {
        // SAFETY: 使用本线程创建的有效 owner 窗口打开剪贴板（NULL owner 会导致
        // EmptyClipboard 后 SetClipboardData 失败，见 OpenClipboard 文档）。
        match unsafe { OpenClipboard(Some(session.owner)) } {
            Ok(()) => {
                session.opened = true;
                return Ok(session);
            }
            Err(error) => {
                last_error = error.to_string();
                if attempt + 1 < OPEN_CLIPBOARD_ATTEMPTS {
                    sleep(OPEN_CLIPBOARD_BACKOFF);
                }
            }
        }
    }
    // session 离开作用域：销毁 owner 窗口（未打开成功，不调用 CloseClipboard）。
    Err(clipboard_error(
        "打开",
        format!("可能正被其他程序长时间占用（{}）", last_error),
    ))
}

/// 解锁全局内存。
///
/// `GlobalUnlock` 的 BOOL 语义：FALSE 且原生 last-error 为 0 表示「本次解锁后完全解锁」，
/// 是正常完成而非失败；windows-rs 的 `BOOL→Result` 包装会把它变成 `Err`（且其 `code`
/// 已被 `nonzero_hresult` 替换为 `S_EMPTY_ERROR`，不能假定 `code()==0`）。
/// 因此调用前显式清零原生 last-error，失败分支直接读取 `GetLastError` 显式判定。
pub(super) fn unlock_moveable(handle: HGLOBAL) -> HalResult<()> {
    // SAFETY: 清零线程本地 last-error，为判定建立确定基线。
    unsafe { SetLastError(WIN32_ERROR(0)) };
    // SAFETY: handle 来自 GlobalAlloc 且处于锁定状态。
    if let Err(error) = unsafe { GlobalUnlock(handle) } {
        let last = unsafe { GetLastError() };
        if last != WIN32_ERROR(0) {
            return Err(clipboard_error(
                "解锁全局内存",
                format!("{}（Win32 错误码 {}）", error, last.0),
            ));
        }
        // last-error == 0：锁计数归零的正常完成，不视为失败。
    }
    Ok(())
}

/// 分配 `GMEM_MOVEABLE` 全局内存并写入字节；任何失败都会释放句柄。
pub(super) fn alloc_moveable(bytes: &[u8]) -> HalResult<HGLOBAL> {
    // SAFETY: 仅调用系统内存 API；长度为 0 时分配 1 字节避免空对象歧义。
    let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) }
        .map_err(|error| clipboard_error("分配全局内存", error))?;
    // SAFETY: handle 由 GlobalAlloc 返回且未被释放。
    let target = unsafe { GlobalLock(handle) };
    if target.is_null() {
        // SAFETY: 分配成功但加锁失败，释放自有句柄，绝不泄漏。
        let _ = unsafe { GlobalFree(Some(handle)) };
        return Err(clipboard_error("锁定全局内存", "GlobalLock 返回空指针"));
    }
    // SAFETY: target 指向至少 bytes.len() 字节的可写内存（0 长度时为 1 字节）。
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), target.cast::<u8>(), bytes.len());
    }
    if let Err(error) = unlock_moveable(handle) {
        // SAFETY: 解锁失败，句柄仍归我们所有，必须释放。
        let _ = unsafe { GlobalFree(Some(handle)) };
        return Err(error);
    }
    Ok(handle)
}

/// 把字节放入指定剪贴板格式。成功后句柄所有权移交系统；失败时释放自有句柄。
fn set_format(action: &str, format: u32, bytes: &[u8]) -> HalResult<()> {
    let handle = alloc_moveable(bytes)?;
    // SAFETY: 句柄来自 GlobalAlloc 且已解锁；HGLOBAL 与 HANDLE 同为指针包装，
    // Option<HANDLE> 是 0.62 的签名形式。
    let handed = unsafe { SetClipboardData(format, Some(HANDLE(handle.0))) };
    match handed {
        Ok(_) => Ok(()), // 所有权已移交系统，不能再 GlobalFree
        Err(error) => {
            // SAFETY: SetClipboardData 失败，句柄仍归我们所有，必须释放。
            let _ = unsafe { GlobalFree(Some(handle)) };
            Err(clipboard_error(action, error))
        }
    }
}

/// 注册自定义剪贴板格式；返回 0 视为失败。
fn register_format(name: &str) -> HalResult<u32> {
    let wide = to_wide(name);
    // SAFETY: wide 是合法的 NUL 结尾 UTF-16 字符串。
    let format = unsafe { RegisterClipboardFormatW(PCWSTR(wide.as_ptr())) };
    if format == 0 {
        return Err(clipboard_error(
            "注册格式",
            format!("RegisterClipboardFormatW(\"{}\") 返回 0", name),
        ));
    }
    Ok(format)
}

/// 写入敏感排除标记：总控标记 + 两个细粒度开关，任一失败都返回错误（由调用方回滚）。
fn set_conceal_marks() -> HalResult<()> {
    for name in [
        EXCLUDE_MONITOR_FORMAT,
        HISTORY_OPT_OUT_FORMAT,
        CLOUD_OPT_OUT_FORMAT,
    ] {
        let format = register_format(name)?;
        set_format("写入敏感排除标记", format, OPT_OUT_FALSE)?;
    }
    Ok(())
}

/// 探测当前剪贴板内容是否带敏感/排除历史标记（任一标记格式在剪贴板上即视为敏感）。
///
/// 只读探测，不打开剪贴板、不修改任何内容；尽力而为，`false` 不代表内容一定不敏感。
pub(super) fn clipboard_content_is_concealed() -> bool {
    [
        EXCLUDE_MONITOR_FORMAT,
        HISTORY_OPT_OUT_FORMAT,
        CLOUD_OPT_OUT_FORMAT,
    ]
    .iter()
    .any(|name| {
        let Ok(format) = register_format(name) else {
            return false;
        };
        // SAFETY: IsClipboardFormatAvailable 是只读查询，无需打开剪贴板。
        unsafe { IsClipboardFormatAvailable(format) }.is_ok()
    })
}

/// 写入敏感文本：`CF_UNICODETEXT` + 剪贴板历史/云同步排除标记，并返回序号所有权凭证。
pub(super) fn write_concealed_text(text: &str) -> HalResult<ClipboardWriteToken> {
    let wide = utf16_with_nul(text)?;
    let bytes: &[u8] = wide_as_bytes(&wide);
    let session = open_clipboard()?;
    // SAFETY: 持有 ClipboardSession（有效 owner + 已打开）期间的剪贴板事务。
    unsafe { EmptyClipboard() }.map_err(|error| clipboard_error("清空", error))?;
    set_format("写入文本", CF_UNICODETEXT.0 as u32, bytes)?;
    if let Err(error) = set_conceal_marks() {
        // 标记失败等于秘密可能被系统历史记录：回滚清空，绝不留下未隐藏的秘密。
        // SAFETY: 同一打开窗口内清空是安全的失败回滚。
        let _ = unsafe { EmptyClipboard() };
        return Err(error);
    }
    // 先关闭剪贴板再采集序号：实测 CloseClipboard 会把本次事务的变更最终计入序号
    // （关闭前读到的是暂态值，用作凭证会永远失配）。关闭后的读取紧邻 drop，竞态窗口
    // 只有几条指令；若此时被其他程序抢先写入，其序号会被记为凭证，属于已知残余竞态。
    drop(session);
    let sequence = unsafe { GetClipboardSequenceNumber() } as i64;
    if sequence == 0 {
        // 系统未维护序号时无法建立所有权凭证：尽力清空并如实报错。
        // （内容已带排除标记，此时清空只是保守收尾，不存在未隐藏的秘密泄漏。）
        if let Ok(session) = open_clipboard() {
            // SAFETY: 持有会话时清空是安全的失败回滚。
            let _ = unsafe { EmptyClipboard() };
            drop(session);
        }
        return Err(clipboard_error(
            "建立所有权凭证",
            "系统未维护剪贴板变更序号",
        ));
    }
    Ok(ClipboardWriteToken { sequence })
}

/// 仅当剪贴板序号仍等于写入后的序号时清空；序号变化（含复制相同文本）一律不触碰。
pub(super) fn clear_if_unchanged(token: &ClipboardWriteToken) -> HalResult<bool> {
    if token.sequence == 0 {
        // 序号 0 不是可靠所有权凭证（写入路径拒绝产生），保守不清除。
        return Ok(false);
    }
    // 先独占打开剪贴板，再比较序号：持有期间其他进程无法修改，检查与清空之间无竞态。
    let session = open_clipboard()?;
    let current = unsafe { GetClipboardSequenceNumber() } as i64;
    if current == 0 || current != token.sequence {
        // 当前序号不可靠或已变化（含用户复制相同文本）→ 不触碰剪贴板。
        drop(session);
        return Ok(false);
    }
    // SAFETY: 已确认序号未变，清空是本凭证约定的动作。
    unsafe { EmptyClipboard() }.map_err(|error| clipboard_error("清空", error))?;
    drop(session);
    Ok(true)
}

/// 把动态 GIF 字节放入系统剪贴板（非敏感，不排除剪贴板历史）。
pub(super) fn write_animated_gif_clipboard(bytes: &[u8]) -> HalResult<()> {
    if bytes.is_empty() {
        // 与 macOS 口径一致的前置校验，不触碰剪贴板。
        return Err(HalError::NativeCallFailed("GIF 内容为空".to_string()));
    }
    let session = open_clipboard()?;
    // SAFETY: 持有 ClipboardSession 的剪贴板事务。
    unsafe { EmptyClipboard() }.map_err(|error| clipboard_error("清空", error))?;
    let primary = register_format(GIF_PRIMARY_FORMAT)?;
    set_format("写入 GIF", primary, bytes)?;
    // 辅助短名尽力而为（对应 macOS 对 public.gif 的宽松写入），失败不影响主格式。
    if let Ok(secondary) = register_format(GIF_SECONDARY_FORMAT) {
        let _ = set_format("写入 GIF 辅助格式", secondary, bytes);
    }
    drop(session);
    Ok(())
}

/// 把 `&[u16]` 重新解释为字节切片（u16 的内存布局即 2 字节/单元）。
fn wide_as_bytes(wide: &[u16]) -> &[u8] {
    // SAFETY: u16 与字节切片长度换算固定（2 字节/单元），仅在本模块内用于内存拷贝。
    unsafe { std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), wide.len() * 2) }
}
