//! Windows 剪贴板所有权与动态 GIF 写入（Win32 DataExchange / Memory / WindowsAndMessaging 真实 API）。
//!
//! - 敏感文本写入 `CF_UNICODETEXT`，同时设置 Windows 剪贴板历史与云同步排除标记，
//!   防止系统级剪贴板历史（Win+V）与云剪贴板记录秘密；
//! - 所有权判定使用 `GetClipboardSequenceNumber` 的系统变更序号，不比较文本内容，
//!   用户之后复制相同文本也不会被误清；
//! - **序号只在证明仍属本次写入后采集**：写入事务关闭后重新独占打开，要求
//!   （a）剪贴板 owner 仍是本次事务的存活消息窗口、（b）本应用私有唯一标记的值与本次一致；
//!   任一不成立即安全放弃（不产出凭证、不触碰剪贴板），绝不认领或清除他人内容；
//! - 剪贴板所有者是本进程按需创建的消息窗口（`HWND_MESSAGE` 子窗口），同一线程创建与销毁（RAII），
//!   不依赖主控台窗口；文档明确：NULL owner 打开后 `EmptyClipboard` 会使 `SetClipboardData` 失败；
//! - `SetClipboardData` 成功后句柄所有权移交系统（不释放）；失败路径一律释放自有句柄。

use crate::{secure::ClipboardWriteToken, HalError, HalResult};
use std::{
    ptr,
    sync::atomic::{AtomicU32, Ordering},
    thread::sleep,
    time::Duration,
};
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{GetLastError, GlobalFree, SetLastError, HANDLE, HGLOBAL, HWND, WIN32_ERROR},
        System::{
            DataExchange::{
                CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardOwner,
                GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard,
                RegisterClipboardFormatW, SetClipboardData,
            },
            Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
            Ole::CF_UNICODETEXT,
        },
        UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE},
    },
};

/// 敏感排除标记（微软文档《Clipboard Formats》→ Cloud Clipboard and Clipboard History Formats
/// 小节给出的准确注册格式名，无空格）：
/// 存在该格式时，剪贴板历史与云同步（漫游剪贴板）都不记录本次内容。
pub(super) const EXCLUDE_MONITOR_FORMAT: &str = "ExcludeClipboardContentFromMonitorProcessing";
/// 细粒度标记：序列化 DWORD 值为 `0`（FALSE）时不进入 Windows 剪贴板历史。
pub(super) const HISTORY_OPT_OUT_FORMAT: &str = "CanIncludeInClipboardHistory";
/// 细粒度标记：序列化 DWORD 值为 `0`（FALSE）时不上传云剪贴板（漫游）。
pub(super) const CLOUD_OPT_OUT_FORMAT: &str = "CanUploadToCloudClipboard";

/// 本应用私有唯一标记格式：载荷为每次写入独有的 8 字节值，用于在重新打开后
/// 证明「当前剪贴板内容仍是本次写入」（比文本比较可靠：同文本的用户复制也带不走/仿不了该标记）。
pub(super) const WRITE_PROBE_FORMAT: &str = "com.allinone.desktop.ClipboardWriteProbe";

/// 动态 GIF 主格式名（MIME 命名，对应 macOS `com.compuserve.gif`）。
pub(super) const GIF_PRIMARY_FORMAT: &str = "image/gif";
/// 动态 GIF 辅助格式名（短名，对应 macOS `public.gif`，尽力而为）。
pub(super) const GIF_SECONDARY_FORMAT: &str = "GIF";

/// 排除标记的载荷：序列化 DWORD `FALSE`（4 字节小端）。内容被系统忽略的标记也统一使用它。
const OPT_OUT_FALSE: &[u8] = &0u32.to_le_bytes();

/// `OpenClipboard` 失败重试次数（剪贴板常被其他进程短暂占用）。
const OPEN_CLIPBOARD_ATTEMPTS: u32 = 10;
/// 每次重试前的等待时间。
const OPEN_CLIPBOARD_BACKOFF: Duration = Duration::from_millis(20);

/// 进程内写入序号（与进程号拼成唯一标记值，跨进程实例也不重合）。
static WRITE_MARKER_COUNTER: AtomicU32 = AtomicU32::new(1);

/// 生成本次写入的私有唯一标记值（8 字节：高 32 位进程号 + 低 32 位自增计数）。
pub(super) fn next_write_marker() -> [u8; 8] {
    let counter = WRITE_MARKER_COUNTER.fetch_add(1, Ordering::Relaxed);
    (((std::process::id() as u64) << 32) | counter as u64).to_le_bytes()
}

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

/// 剪贴板事务用的所有者窗口：系统预定义 `STATIC` 类 + `HWND_MESSAGE` 父窗口，
/// 不注册窗口类、不显示、不依赖任何既有窗口；同一线程创建，离开作用域自动销毁（含 panic 展开）。
pub(super) struct OwnerWindow {
    hwnd: HWND,
}

impl OwnerWindow {
    pub(super) fn new() -> HalResult<Self> {
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
        Ok(Self { hwnd })
    }

    pub(super) fn handle(&self) -> HWND {
        self.hwnd
    }
}

impl Drop for OwnerWindow {
    fn drop(&mut self) {
        // SAFETY: 窗口由本线程创建，句柄未被销毁过。
        let _ = unsafe { DestroyWindow(self.hwnd) };
    }
}

/// 剪贴板打开会话：与 `OwnerWindow` 配对使用，离开作用域自动 `CloseClipboard`
/// （含错误提前返回与 panic 展开；打开与关闭必然同线程）。
struct ClipboardSession<'w> {
    _owner: &'w OwnerWindow,
}

impl Drop for ClipboardSession<'_> {
    fn drop(&mut self) {
        // SAFETY: 会话存活期间剪贴板处于打开状态，关闭与打开在同一线程配对。
        let _ = unsafe { CloseClipboard() };
    }
}

/// 在给定 owner 窗口上打开系统剪贴板，被占用时退避重试；持续失败返回明确错误。
fn open_clipboard_on(owner: &OwnerWindow) -> HalResult<ClipboardSession<'_>> {
    let mut last_error = String::new();
    for attempt in 0..OPEN_CLIPBOARD_ATTEMPTS {
        // SAFETY: 使用本线程创建的有效 owner 窗口打开剪贴板（NULL owner 会导致
        // EmptyClipboard 后 SetClipboardData 失败，见 OpenClipboard 文档）。
        match unsafe { OpenClipboard(Some(owner.handle())) } {
            Ok(()) => return Ok(ClipboardSession { _owner: owner }),
            Err(error) => {
                last_error = error.to_string();
                if attempt + 1 < OPEN_CLIPBOARD_ATTEMPTS {
                    sleep(OPEN_CLIPBOARD_BACKOFF);
                }
            }
        }
    }
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
/// 因此调用前显式清零原生 last-error，失败分支直接读取 `GetLastError` 显式判定
/// （实测真实失败为 `ERROR_NOT_LOCKED(158)`）。
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

/// 读取指定格式的字节载荷（调用方需持有打开的剪贴板会话）。
fn read_format_bytes(format: u32) -> Option<Vec<u8>> {
    // SAFETY: 剪贴板处于打开状态；读取到的句柄归剪贴板所有，只读不释放。
    let handle = unsafe { GetClipboardData(format) }.ok()?;
    let hglobal = HGLOBAL(handle.0);
    unsafe {
        let size = GlobalSize(hglobal);
        let view = GlobalLock(hglobal);
        if view.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(view.cast::<u8>(), size).to_vec();
        let _ = unlock_moveable(hglobal);
        Some(bytes)
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
///
/// 保守说明：`CanIncludeInClipboardHistory` / `CanUploadToCloudClipboard` 只要在剪贴板上就视为敏感，
/// 即使其 DWORD 值为 `1`（显式允许记录/同步）——载荷值只有打开剪贴板才能读取，
/// 本探测刻意不打开剪贴板（监听热路径），因此可能跳过少量可记录内容，但绝不会收录秘密。
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

/// 写入敏感文本（生产入口）：写入 + 所有权验证 + 最终序号采集。
pub(super) fn write_concealed_text(text: &str) -> HalResult<ClipboardWriteToken> {
    write_concealed_text_with_hook(text, || {})
}

/// 写入敏感文本；`after_close_before_verify` 在「事务关闭之后、所有权验证之前」执行。
///
/// 仅供回归测试**确定性**注入「关闭后、采序号前被另一次复制抢占」场景；生产传空闭包。
pub(super) fn write_concealed_text_with_hook(
    text: &str,
    after_close_before_verify: impl FnOnce(),
) -> HalResult<ClipboardWriteToken> {
    let wide = utf16_with_nul(text)?;
    let bytes: &[u8] = wide_as_bytes(&wide);
    let marker = next_write_marker();
    // owner 窗口在验证完成前保持存活（Drop 负责销毁；panic 展开同样回收）。
    let owner = OwnerWindow::new()?;
    {
        // 第一段事务：写入内容 + 排除标记 + 私有唯一标记，然后关闭。
        // 实测 CloseClipboard 才把本事务的变更最终计入序号，因此序号不在这里采集。
        let session = open_clipboard_on(&owner)?;
        // SAFETY: 持有 ClipboardSession（有效 owner + 已打开）期间的剪贴板事务。
        unsafe { EmptyClipboard() }.map_err(|error| clipboard_error("清空", error))?;
        set_format("写入文本", CF_UNICODETEXT.0 as u32, bytes)?;
        if let Err(error) = set_conceal_marks() {
            // 标记失败等于秘密可能被系统历史记录：回滚清空，绝不留下未隐藏的秘密。
            // SAFETY: 同一打开窗口内清空是安全的失败回滚（此刻内容只可能是本次写入）。
            let _ = unsafe { EmptyClipboard() };
            return Err(error);
        }
        let probe_format = register_format(WRITE_PROBE_FORMAT)?;
        if let Err(error) = set_format("写入所有权标记", probe_format, &marker) {
            // SAFETY: 同一打开窗口内清空是安全的失败回滚。
            let _ = unsafe { EmptyClipboard() };
            return Err(error);
        }
        drop(session); // 关闭剪贴板（owner 窗口仍存活）
    }

    // 测试钩子：确定性抢占注入点（生产为空）。
    after_close_before_verify();

    // 第二段事务：重新独占打开，证明「当前内容仍是本次写入」后才采集最终序号。
    // 打开期间其他进程无法修改剪贴板，验证与采集之间无竞态；
    // 若在关闭后的窗口里被其他程序抢占，验证必然失败并安全放弃。
    let session = open_clipboard_on(&owner)?;
    let still_ours = clipboard_owner_is(&owner) && probe_matches(WRITE_PROBE_FORMAT, &marker);
    if !still_ours {
        // 被其他程序抢先修改（含复制相同文本）：安全放弃——不产出凭证、不触碰剪贴板。
        return Err(clipboard_error(
            "建立所有权凭证",
            "剪贴板在写入后被其他程序修改，已放弃所有权凭证",
        ));
    }
    let sequence = unsafe { GetClipboardSequenceNumber() } as i64;
    if sequence == 0 {
        // 系统未维护序号时无法建立所有权凭证。此刻已在独占会话内验证仍属本次写入，
        // 清空只可能删除本次内容，绝不会误清后来的复制。
        // SAFETY: 已验证仍是本次写入，清空是安全的失败收尾。
        let _ = unsafe { EmptyClipboard() };
        return Err(clipboard_error(
            "建立所有权凭证",
            "系统未维护剪贴板变更序号",
        ));
    }
    drop(session);
    Ok(ClipboardWriteToken { sequence })
}

/// 剪贴板 owner 是否仍是本次事务的窗口（他人 EmptyClipboard 会转移所有权）。
fn clipboard_owner_is(owner: &OwnerWindow) -> bool {
    // SAFETY: 只读查询。
    matches!(unsafe { GetClipboardOwner() }, Ok(hwnd) if hwnd == owner.handle())
}

/// 私有唯一标记是否仍等于本次写入的值（调用方需持有打开的剪贴板会话）。
fn probe_matches(format_name: &str, marker: &[u8; 8]) -> bool {
    let Ok(format) = register_format(format_name) else {
        return false;
    };
    matches!(read_format_bytes(format), Some(bytes) if bytes == marker)
}

/// 仅当剪贴板序号仍等于写入后的序号时清空；序号变化（含复制相同文本）一律不触碰。
pub(super) fn clear_if_unchanged(token: &ClipboardWriteToken) -> HalResult<bool> {
    if token.sequence == 0 {
        // 序号 0 不是可靠所有权凭证（写入路径拒绝产生），保守不清除。
        return Ok(false);
    }
    // 先独占打开剪贴板，再比较序号：持有期间其他进程无法修改，检查与清空之间无竞态。
    let owner = OwnerWindow::new()?;
    let session = open_clipboard_on(&owner)?;
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
    let owner = OwnerWindow::new()?;
    let session = open_clipboard_on(&owner)?;
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
