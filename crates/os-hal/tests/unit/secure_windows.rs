//! Windows 剪贴板所有权与凭据管理器子实现的纯逻辑回归单测。
//!
//! 默认用例不读写用户真实剪贴板、不访问凭据管理器、不弹权限框；
//! 真实剪贴板的集成验证（含确定性抢占回归）以 `#[ignore]` 提供，需在桌面会话中显式运行。
use super::{clipboard_win, secret_win, write_animated_gif_clipboard};
use crate::HalError;
use std::sync::{Mutex, MutexGuard};
use windows::Win32::{
    Foundation::{HANDLE, HGLOBAL, HWND},
    System::{
        DataExchange::{
            CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
        },
        Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
        Ole::CF_UNICODETEXT,
    },
    UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE,
    },
};

#[test]
fn utf16_conversion_terminates_and_round_trips_non_ascii() {
    let wide = clipboard_win::utf16_with_nul("密码🔑abc").unwrap();
    // 结尾必须有 NUL 终止符。
    assert_eq!(wide.last(), Some(&0u16));
    // 去掉终止符后解码应与原文一致（覆盖 BMP 与代理对）。
    let round_trip = String::from_utf16(&wide[..wide.len() - 1]).unwrap();
    assert_eq!(round_trip, "密码🔑abc");
    // 空文本合法（只含终止符）。
    assert_eq!(clipboard_win::utf16_with_nul("").unwrap(), vec![0u16]);
}

#[test]
fn text_with_interior_nul_is_rejected_before_touching_clipboard() {
    let error = clipboard_win::utf16_with_nul("a\0b").unwrap_err();
    assert!(
        error.to_string().contains("NUL"),
        "必须拒绝会被 Windows 读者截断的文本：{}",
        error
    );
}

#[test]
fn concealed_marker_and_gif_format_names_are_the_documented_windows_values() {
    // 锁定微软文档《Clipboard Formats》→ Cloud Clipboard and Clipboard History Formats
    // 小节给出的准确注册格式名（无空格），防止漂移导致秘密进入剪贴板历史/云同步。
    assert_eq!(
        clipboard_win::EXCLUDE_MONITOR_FORMAT,
        "ExcludeClipboardContentFromMonitorProcessing"
    );
    assert_eq!(
        clipboard_win::HISTORY_OPT_OUT_FORMAT,
        "CanIncludeInClipboardHistory"
    );
    assert_eq!(
        clipboard_win::CLOUD_OPT_OUT_FORMAT,
        "CanUploadToCloudClipboard"
    );
    // 三个排除格式名都不得含空格（带空格的旧名 Windows 不认识，等于没有排除标记）。
    for name in [
        clipboard_win::EXCLUDE_MONITOR_FORMAT,
        clipboard_win::HISTORY_OPT_OUT_FORMAT,
        clipboard_win::CLOUD_OPT_OUT_FORMAT,
    ] {
        assert!(!name.contains(' '), "注册格式名不得含空格：{}", name);
    }
    // 私有所有权标记格式名（本应用命名空间内）。
    assert_eq!(
        clipboard_win::WRITE_PROBE_FORMAT,
        "com.allinone.desktop.ClipboardWriteProbe"
    );
    // GIF 主/辅格式名与 macOS 双 UTI 写入口径对应。
    assert_eq!(clipboard_win::GIF_PRIMARY_FORMAT, "image/gif");
    assert_eq!(clipboard_win::GIF_SECONDARY_FORMAT, "GIF");
}

#[test]
fn write_probe_markers_are_unique_and_fixed_width() {
    // 私有唯一标记是所有权验证的依据：每次写入必须不同、固定 8 字节。
    let first = clipboard_win::next_write_marker();
    let second = clipboard_win::next_write_marker();
    assert_eq!(first.len(), 8);
    assert_eq!(second.len(), 8);
    assert_ne!(first, second, "每次写入的私有标记值必须不同");
}

#[test]
fn credential_target_names_are_confined_to_app_namespace() {
    // 合法：应用命名空间内的 service 映射为 {service}/{account}。
    assert_eq!(
        secret_win::credential_target_name("com.allinone.desktop.translator", "deepl").unwrap(),
        "com.allinone.desktop.translator/deepl"
    );
    // 拒绝命名空间外的 service，保证绝不触碰用户已有的其他凭据。
    let outside = secret_win::credential_target_name("deepl", "user").unwrap_err();
    assert!(outside.to_string().contains("命名空间"), "{}", outside);
    let spoof = secret_win::credential_target_name("evil.com.allinone.desktop", "x").unwrap_err();
    assert!(spoof.to_string().contains("命名空间"), "{}", spoof);
    assert!(secret_win::credential_target_name("com.allinone.desktop-other", "x").is_err());
    assert!(secret_win::credential_target_name("com.allinone.desktopOther", "x").is_err());
}

#[test]
fn credential_target_name_rejects_ambiguous_or_empty_parts() {
    // 空片段。
    assert!(secret_win::credential_target_name("", "a").is_err());
    assert!(secret_win::credential_target_name("com.allinone.desktop.a", "").is_err());
    // '/' 与 TargetName 分隔符冲突，会造成条目歧义。
    assert!(secret_win::credential_target_name("com.allinone.desktop.a", "b/c").is_err());
    assert!(secret_win::credential_target_name("com.allinone.desktop.a/b", "c").is_err());
    // NUL 会截断 C 字符串，可能读到别的条目。
    assert!(secret_win::credential_target_name("com.allinone.desktop.a", "b\0c").is_err());
    // 超过通用 TargetName 的 UTF-16 长度上限（32767 单元）。
    let long_account = "a".repeat(40_000);
    assert!(secret_win::credential_target_name("com.allinone.desktop.a", &long_account).is_err());
}

#[test]
fn credential_blob_size_limit_is_enforced() {
    // 二进制秘密按原样存取，只限制长度：2560 上限内合法，超限明确报错。
    assert!(secret_win::validate_secret_len(0).is_ok());
    assert!(secret_win::validate_secret_len(2560).is_ok());
    let error = secret_win::validate_secret_len(2561).unwrap_err();
    assert!(error.to_string().contains("2560"), "{}", error);
}

#[test]
fn win32_error_codes_map_to_actionable_hal_errors() {
    // HRESULT_FROM_WIN32 形式提取低 16 位错误码。
    assert_eq!(secret_win::win32_error_code(0x80070490u32 as i32), 1168);
    assert_eq!(secret_win::win32_error_code(0x80070005u32 as i32), 5);
    // 拒绝访问 → 权限错误（带能力名与指引）。
    match secret_win::cred_error("写入", 5) {
        HalError::PermissionDenied {
            capability,
            guidance,
        } => {
            assert_eq!(capability, "credential_manager");
            assert!(guidance.contains("写入"));
        }
        other => panic!("ERROR_ACCESS_DENIED 应映射为 PermissionDenied：{}", other),
    }
    // 其余错误 → 原生调用失败，信息含动作与错误码但不含秘密。
    match secret_win::cred_error("读取", 1317) {
        HalError::NativeCallFailed(message) => {
            assert!(message.contains("读取"), "{}", message);
            assert!(message.contains("1317"), "{}", message);
        }
        other => panic!("一般 Win32 错误应映射为 NativeCallFailed：{}", other),
    }
}

#[test]
fn gif_write_rejects_empty_payload_without_touching_clipboard() {
    // 空内容在任何剪贴板操作之前被拒绝（与 macOS 口径一致）。
    let error = write_animated_gif_clipboard(&[]).unwrap_err();
    assert!(error.contains("GIF 内容为空"), "{}", error);
    let inner = clipboard_win::write_animated_gif_clipboard(&[]).unwrap_err();
    assert!(inner.to_string().contains("GIF 内容为空"), "{}", inner);
}

#[test]
fn alloc_moveable_round_trips_bytes_and_unlock_completes_normally() {
    // 覆盖 GlobalUnlock 的两条判定路径（实测语义）：
    // - 正常完成解锁：FALSE 且 last-error=0 → 必须视为成功，绝不允许误报失败；
    // - 真实失败：对已解锁句柄再次解锁，FALSE 且 last-error=ERROR_NOT_LOCKED(158) → Err。
    let payload: &[u8] = &[0x00, 0x41, 0xFF, 0x7F, 0x00, 0x42];
    let handle: HGLOBAL = clipboard_win::alloc_moveable(payload).unwrap();
    // SAFETY: handle 有效且未释放；重新加锁读回校验，再显式解锁、释放。
    unsafe {
        let view = GlobalLock(handle);
        assert!(!view.is_null());
        let size = GlobalSize(handle);
        assert!(size >= payload.len(), "分配尺寸 {} 不足载荷长度", size);
        let bytes = std::slice::from_raw_parts(view.cast::<u8>(), payload.len());
        assert_eq!(bytes, payload);
        // 首次解锁（锁计数归零）是「FALSE + last-error 0」的正常完成分支。
        assert!(clipboard_win::unlock_moveable(handle).is_ok());
        // 再次解锁是真实失败（ERROR_NOT_LOCKED），必须报错而不能当作正常完成。
        assert!(clipboard_win::unlock_moveable(handle).is_err());
        // GlobalFree 成功返回 NULL；windows-rs 的 Result 包装把 NULL 映射为 Err、
        // 把未释放句柄映射为 Ok，语义与直觉相反——按其真实语义断言（Err = 释放成功）。
        let free = windows::Win32::Foundation::GlobalFree(Some(handle));
        assert!(
            free.is_err(),
            "GlobalFree 释放成功（返回 NULL）在 windows-rs 包装中应表现为 Err：{:?}",
            free
        );
    }
}

#[test]
fn owner_window_lifecycle_is_self_managed() {
    // 消息窗口生命周期自管理：RAII 创建/销毁，不依赖主控台或任何外部窗口。
    let owner = clipboard_win::OwnerWindow::new().unwrap();
    assert!(!owner.handle().is_invalid());
    // 显式销毁（Drop 语义），供退出/异常路径复用同一回收方式。
    drop(owner);
}

// ---------------------------------------------------------------------------
// 真实剪贴板集成验证（显式 ignored）：确定性触发「关闭后、采序号前被抢占」。
// ---------------------------------------------------------------------------

/// 剪贴板真实测试互斥执行（前一用例 panic 毒化锁时继续）。
static CLIPBOARD_GATE: Mutex<()> = Mutex::new(());

fn clipboard_gate() -> MutexGuard<'static, ()> {
    CLIPBOARD_GATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 测试侧独立的剪贴板会话：自建消息窗口作为 owner（NULL owner 会使写入失败）。
struct TestClipboard {
    owner: HWND,
}

impl TestClipboard {
    fn open() -> Self {
        // SAFETY: 纯消息窗口，测试线程内创建并销毁。
        let owner = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                windows::core::w!("secure-windows-unit-test-owner"),
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
        .expect("创建测试用所有者窗口失败");
        unsafe { OpenClipboard(Some(owner)).expect("打开系统剪贴板失败") };
        Self { owner }
    }
}

impl Drop for TestClipboard {
    fn drop(&mut self) {
        // SAFETY: 与本会话的打开/创建配对。
        unsafe {
            let _ = CloseClipboard();
            let _ = DestroyWindow(self.owner);
        }
    }
}

/// 模拟「另一个程序」的完整复制（Open → Empty → Set → Close），确定性抢占注入用。
fn external_write_text(text: &str) {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = unsafe { std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), wide.len() * 2) };
    let board = TestClipboard::open();
    unsafe {
        EmptyClipboard().expect("清空系统剪贴板失败");
        let handle = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).expect("分配全局内存失败");
        let target = GlobalLock(handle);
        assert!(!target.is_null());
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), target.cast::<u8>(), bytes.len());
        let _ = GlobalUnlock(handle);
        SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(handle.0)))
            .expect("SetClipboardData 失败");
    }
    drop(board);
}

/// 测试侧读取当前 `CF_UNICODETEXT` 文本（独立于被测实现）。
fn external_read_text() -> Option<String> {
    // 会话只用于保持剪贴板打开（Drop 时关闭）。
    let _board = TestClipboard::open();
    // SAFETY: 打开状态下读取剪贴板数据句柄。
    let handle = unsafe { GetClipboardData(CF_UNICODETEXT.0 as u32) }.ok()?;
    let hglobal = HGLOBAL(handle.0);
    unsafe {
        let size = GlobalSize(hglobal);
        let view = GlobalLock(hglobal);
        if view.is_null() {
            return None;
        }
        let units = size / 2;
        let wide = std::slice::from_raw_parts(view.cast::<u16>(), units);
        let len = wide.iter().position(|&u| u == 0).unwrap_or(units);
        let text = String::from_utf16_lossy(&wide[..len]);
        let _ = GlobalUnlock(hglobal);
        Some(text)
    }
}

/// 测试侧清空剪贴板（用例收尾，避免残留虚构内容）。
fn external_clear() {
    let board = TestClipboard::open();
    unsafe { EmptyClipboard().expect("清空系统剪贴板失败") };
    drop(board);
}

#[test]
#[ignore = "会覆盖真实系统剪贴板内容，需显式运行"]
fn preemption_between_close_and_capture_is_abandoned_and_never_cleared() {
    let _gate = clipboard_gate();
    // 钩子在「事务关闭之后、所有权验证之前」确定性注入另一次复制（不依赖概率）。
    let result = clipboard_win::write_concealed_text_with_hook("ours-secret", || {
        external_write_text("theirs-later-copy");
    });
    assert!(
        result.is_err(),
        "抢占后必须安全放弃，不得产出所有权凭证：{:?}",
        result
    );
    // 外来内容必须原样保留（不认领、不清除他人内容）。
    assert_eq!(external_read_text().as_deref(), Some("theirs-later-copy"));
    external_clear();
}

#[test]
#[ignore = "会覆盖真实系统剪贴板内容，需显式运行"]
fn identical_text_copied_after_close_is_still_preemption_and_never_cleared() {
    use crate::{ClipboardOwnership, NativeOsHal};
    let _gate = clipboard_gate();
    let native = NativeOsHal::new();
    // 先正常写入拿到旧凭证（同时验证正常路径的所有权验证与序号采集）。
    let old_token = native.write_concealed_text("same-text").unwrap();
    // 「关闭后、采序号前」用户复制了完全相同的文本：私有标记/owner 验证必须识别为抢占。
    let result = clipboard_win::write_concealed_text_with_hook("same-text", || {
        external_write_text("same-text");
    });
    assert!(result.is_err(), "同文本抢占也必须安全放弃：{:?}", result);
    assert_eq!(external_read_text().as_deref(), Some("same-text"));
    // 旧凭证也不得清除用户之后的同文本复制（序号已变化）。
    assert!(!native.clear_if_unchanged(&old_token).unwrap());
    assert_eq!(external_read_text().as_deref(), Some("same-text"));
    external_clear();
}

#[test]
fn dibv5_is_bottom_up_bgra_and_rejects_a_short_buffer() {
    let rgba = [1u8, 2, 3, 4, 5, 6, 7, 8];
    let dib = clipboard_win::rgba_to_dibv5(1, 2, &rgba).unwrap();
    assert_eq!(dib.len(), 124 + 8);
    assert_eq!(u32::from_le_bytes(dib[0..4].try_into().unwrap()), 124);
    assert_eq!(i32::from_le_bytes(dib[4..8].try_into().unwrap()), 1);
    assert_eq!(i32::from_le_bytes(dib[8..12].try_into().unwrap()), 2);
    assert_eq!(u16::from_le_bytes(dib[14..16].try_into().unwrap()), 32);
    // 正高度是自下而上，第一个像素来自源图像最后一行。
    assert_eq!(&dib[124..128], &[7, 6, 5, 8]);
    assert_eq!(&dib[128..132], &[3, 2, 1, 4]);
    assert!(clipboard_win::rgba_to_dibv5(2, 2, &rgba).is_err());
}

#[test]
#[ignore = "会覆盖真实系统剪贴板，需显式运行；覆盖全屏尺寸的 1418 回归"]
fn fullscreen_rgba_clipboard_write_survives_where_null_owner_fails() {
    use windows::Win32::System::{DataExchange::IsClipboardFormatAvailable, Ole::CF_DIBV5};
    let _gate = clipboard_gate();
    let width = 1920u32;
    let height = 1080u32;
    let rgba = vec![12u8; width as usize * height as usize * 4];
    clipboard_win::write_rgba_clipboard(width, height, &rgba).expect("全屏 RGBA 写入剪贴板");
    let available = unsafe { IsClipboardFormatAvailable(CF_DIBV5.0 as u32) };
    assert!(available.is_ok(), "写入后剪贴板必须带有 CF_DIBV5");
}
