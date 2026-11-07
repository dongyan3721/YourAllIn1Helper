//! Windows 凭据管理器（Credential Manager）的 SecretStore 原生实现。
//!
//! - 只在本应用命名空间（`com.allinone.desktop*`）内读写删通用凭据，
//!   TargetName 统一为 `{service}/{account}`，绝不触碰用户已有的其他凭据；
//! - `CRED_TYPE_GENERIC` + `CRED_PERSIST_LOCAL_MACHINE`（仅本机持久化，不随账户漫游）；
//! - 这是普通用户凭据存储：受当前用户账户保护，**不是** Windows Hello/生物识别保护，
//!   也不绑定受保护的设备密钥（那是密码本模块的独立需求）；
//! - 秘密按二进制 blob 原样存取（`CredentialBlob`），不假定 UTF-8；
//! - 错误路径：条目缺失按契约返回 `Ok(None)` / 删除成功，拒绝访问归为权限错误，
//!   错误信息绝不包含秘密内容。

use crate::{HalError, HalResult};
use std::ptr;
use windows::{
    core::PCWSTR,
    Win32::Security::Credentials::{
        CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_MAX_CREDENTIAL_BLOB_SIZE,
        CRED_MAX_GENERIC_TARGET_NAME_LENGTH, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
    },
};

/// 本应用凭据命名空间前缀；`service` 必须以它开头，保证只操作自家条目。
pub(super) const APP_NAMESPACE: &str = "com.allinone.desktop";

/// 条目注释（显示在凭据管理器界面，固定短串，无用户内容）。
const TARGET_COMMENT: &str = "All-in-One Toolbox";

/// `ERROR_NOT_FOUND`：读取/删除时条目不存在。
const ERROR_NOT_FOUND: u32 = 1168;
/// `ERROR_ACCESS_DENIED`：系统策略或权限拒绝。
const ERROR_ACCESS_DENIED: u32 = 5;

/// 从 Win32 HRESULT 提取低 16 位错误码（`HRESULT_FROM_WIN32` 形式的错误）。
pub(super) fn win32_error_code(hresult: i32) -> u32 {
    hresult as u32 & 0xFFFF
}

/// 校验 service / account 片段：非空、不含 `/`（TargetName 分隔符，避免歧义）、不含 NUL。
fn validate_name_part(part: &str, what: &str) -> HalResult<()> {
    if part.is_empty() {
        return Err(HalError::NativeCallFailed(format!(
            "凭据{}不能为空",
            what
        )));
    }
    if part.contains('/') {
        return Err(HalError::NativeCallFailed(format!(
            "凭据{}不能包含 '/'（与 TargetName 分隔符冲突）",
            what
        )));
    }
    if part.chars().any(|c| c == '\0') {
        return Err(HalError::NativeCallFailed(format!(
            "凭据{}不能包含 NUL 字符",
            what
        )));
    }
    Ok(())
}

/// 构造并校验凭据 TargetName：`{service}/{account}`，限制在本应用命名空间内。
pub(super) fn credential_target_name(service: &str, account: &str) -> HalResult<String> {
    validate_name_part(service, "服务名")?;
    validate_name_part(account, "账户名")?;
    if !service.starts_with(APP_NAMESPACE) {
        return Err(HalError::NativeCallFailed(format!(
            "凭据服务名必须以本应用命名空间 '{}' 开头（实际：{}）",
            APP_NAMESPACE, service
        )));
    }
    let target = format!("{}/{}", service, account);
    if target.encode_utf16().count() > CRED_MAX_GENERIC_TARGET_NAME_LENGTH as usize {
        return Err(HalError::NativeCallFailed(format!(
            "凭据 TargetName 超过 {} 个 UTF-16 单元上限",
            CRED_MAX_GENERIC_TARGET_NAME_LENGTH
        )));
    }
    Ok(target)
}

/// 校验秘密 blob 长度不超过通用凭据上限（2560 字节），二进制内容不做解释。
pub(super) fn validate_secret_len(len: usize) -> HalResult<()> {
    if len > CRED_MAX_CREDENTIAL_BLOB_SIZE as usize {
        return Err(HalError::NativeCallFailed(format!(
            "秘密长度 {} 超过凭据管理器通用凭据上限 {} 字节",
            len, CRED_MAX_CREDENTIAL_BLOB_SIZE
        )));
    }
    Ok(())
}

/// 凭据管理器错误归类：拒绝访问 → 权限错误；其余为原生调用失败；信息不含秘密。
pub(super) fn cred_error(action: &str, code: u32) -> HalError {
    match code {
        ERROR_ACCESS_DENIED => HalError::PermissionDenied {
            capability: "credential_manager",
            guidance: format!(
                "Windows 凭据管理器拒绝{}（Win32 错误码 {}）。请检查系统策略或当前用户权限后重试",
                action, code
            ),
        },
        _ => HalError::NativeCallFailed(format!(
            "Windows 凭据管理器{}失败（Win32 错误码 {}）",
            action, code
        )),
    }
}

/// 字符串转 NUL 结尾 UTF-16（调用方已通过 `validate_name_part` 保证不含 NUL）。
fn to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 读取条目；不存在返回 `Ok(None)`。
pub(super) fn get_secret(service: &str, account: &str) -> HalResult<Option<Vec<u8>>> {
    let target = credential_target_name(service, account)?;
    let target_wide = to_wide(&target);
    let mut raw: *mut CREDENTIALW = ptr::null_mut();
    // SAFETY: target_wide 是合法 NUL 结尾字符串；raw 仅作输出。
    let result = unsafe {
        CredReadW(
            PCWSTR(target_wide.as_ptr()),
            CRED_TYPE_GENERIC,
            None,
            &mut raw,
        )
    };
    match result {
        Ok(()) => {
            if raw.is_null() {
                return Ok(None);
            }
            // SAFETY: CredReadW 成功时 raw 指向有效 CREDENTIALW；拷出 blob 后立即 CredFree。
            let blob = unsafe {
                let cred = &*raw;
                let bytes = if cred.CredentialBlob.is_null() || cred.CredentialBlobSize == 0 {
                    Vec::new()
                } else {
                    std::slice::from_raw_parts(
                        cred.CredentialBlob,
                        cred.CredentialBlobSize as usize,
                    )
                    .to_vec()
                };
                CredFree(raw.cast());
                bytes
            };
            Ok(Some(blob))
        }
        Err(error) => {
            let code = win32_error_code(error.code().0);
            if code == ERROR_NOT_FOUND {
                Ok(None)
            } else {
                Err(cred_error("读取", code))
            }
        }
    }
}

/// 写入或覆盖条目（二进制秘密原样存储）。
pub(super) fn set_secret(service: &str, account: &str, secret: &[u8]) -> HalResult<()> {
    let target = credential_target_name(service, account)?;
    validate_secret_len(secret.len())?;
    let target_wide = to_wide(&target);
    let comment_wide = to_wide(TARGET_COMMENT);
    // SAFETY: 所有指针字段指向本函数栈上存活的缓冲；CredWriteW 同步拷贝数据。
    let mut cred = CREDENTIALW::default();
    cred.Type = CRED_TYPE_GENERIC;
    cred.TargetName = windows::core::PWSTR(target_wide.as_ptr() as *mut u16);
    cred.Comment = windows::core::PWSTR(comment_wide.as_ptr() as *mut u16);
    cred.CredentialBlobSize = secret.len() as u32;
    cred.CredentialBlob = secret.as_ptr() as *mut u8;
    cred.Persist = CRED_PERSIST_LOCAL_MACHINE;
    // SAFETY: cred 结构完整且字段内存有效。
    unsafe { CredWriteW(&cred, 0) }.map_err(|error| {
        cred_error("写入", win32_error_code(error.code().0))
    })
}

/// 删除条目；不存在视为成功。
pub(super) fn delete_secret(service: &str, account: &str) -> HalResult<()> {
    let target = credential_target_name(service, account)?;
    let target_wide = to_wide(&target);
    // SAFETY: target_wide 是合法 NUL 结尾字符串。
    match unsafe { CredDeleteW(PCWSTR(target_wide.as_ptr()), CRED_TYPE_GENERIC, None) } {
        Ok(()) => Ok(()),
        Err(error) => {
            let code = win32_error_code(error.code().0);
            if code == ERROR_NOT_FOUND {
                Ok(())
            } else {
                Err(cred_error("删除", code))
            }
        }
    }
}
