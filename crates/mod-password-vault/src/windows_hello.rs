//! Windows Hello 设备密钥。
//!
//! 安全边界（以 Windows SDK 10.0.26100 的 `ncrypt.h` 为准）：
//! - DKEK 用 Microsoft Passport Key Storage Provider 中不可导出的 RSA-2048 公钥包装随机 DKEK。
//!   Passport 本机只支持 RSAES-PKCS1-v1_5，新信封显式使用 v3；旧 v1 OAEP 只读。
//! - 解开 DKEK 的唯一路径是 `NCryptDecrypt`。调用前设置官方属性
//!   `NCRYPT_PIN_CACHE_IS_GESTURE_REQUIRED_PROPERTY`，不向 NCryptDecrypt 传入与其不匹配的缓存标志。
//! - 登记时先用 `NCRYPT_SILENT_FLAG` 解密一段随机探测值。只有返回 `NTE_SILENT_CONTEXT`
//!   才说明这次私钥操作必须弹出系统界面。静默解密一旦成功，立即删除密钥并报告不可用。
//! - `UserConsentVerifier.CheckAvailabilityAsync` 只用来判断 Hello 是否已配置，不参与释放密钥。
//!   不调用同意验证器的验证请求，也不把验证成功当成读取 DPAPI 或凭据管理器的条件。
//! - Windows Hello 手势包含人脸、指纹或 PIN。SDK 没有等价于 macOS `biometryCurrentSet`
//!   的“拒绝 PIN、生物识别集合变化后失效”标志，因此本后端不得宣称生物识别等价。
//! - 提供程序、算法、策略或静默探测不满足上述边界时返回不可用，主密码继续有效。
use crate::{
    error::{VaultError, VaultResult},
    hello_store::{HelloBackend, WindowsHelloKeyStore, OAEP_LEN, PKCS1_PREFIX},
    keystore::VaultKeyStore,
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicIsize, Ordering},
        Arc,
    },
};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

const NTE_INVALID_PARAMETER: i32 = 0x8009_0027_u32 as i32;
const NTE_BAD_DATA: i32 = 0x8009_0005_u32 as i32;
const NTE_BAD_FLAGS: i32 = 0x8009_0009_u32 as i32;
const NTE_NO_KEY: i32 = 0x8009_000D_u32 as i32;
const NTE_PERM: i32 = 0x8009_0010_u32 as i32;
const NTE_BAD_KEYSET: i32 = 0x8009_0016_u32 as i32;
const NTE_SILENT_CONTEXT: i32 = 0x8009_0022_u32 as i32;
const NTE_NOT_SUPPORTED: i32 = 0x8009_0029_u32 as i32;
const NTE_USER_CANCELLED: i32 = 0x8009_0036_u32 as i32;
const ERROR_CANCELLED: i32 = 0x8007_04C7_u32 as i32;
const WINBIO_E_CANCELED: i32 = 0x8009_8004_u32 as i32;
const WINBIO_E_NO_MATCH: i32 = 0x8009_8005_u32 as i32;
const WINBIO_E_DISABLED: i32 = 0x8009_8032_u32 as i32;
const WINBIO_E_CRED_PROV_DISABLED: i32 = 0x8009_8030_u32 as i32;
const WINBIO_E_CRED_PROV_SECURITY_LOCKOUT: i32 = 0x8009_8047_u32 as i32;
const OAEP_LABEL: &[u8] = b"allinone.vault.dkek.v1";
const PROMPT: &str = "解锁密码本设备密钥";

/// 静默解密探测的三种结果。只有“必须显示界面”可以继续保存 DKEK。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SilentDecrypt {
    PlaintextReturned,
    GestureRequired,
    Failed,
}

pub(crate) fn gesture_gate_holds(result: SilentDecrypt) -> bool {
    matches!(result, SilentDecrypt::GestureRequired)
}

/// 静默探测只看原始 HRESULT。`None` 表示解密成功，不能当成已要求手势。
/// 除精确的 `NTE_SILENT_CONTEXT` 以外，包括 `NTE_BAD_FLAGS` 和 `NTE_NOT_SUPPORTED`，一律失败。
pub(crate) fn classify_silent_hresult(code: Option<i32>) -> SilentDecrypt {
    match code {
        None => SilentDecrypt::PlaintextReturned,
        Some(NTE_SILENT_CONTEXT) => SilentDecrypt::GestureRequired,
        Some(_) => SilentDecrypt::Failed,
    }
}

pub(crate) fn map_ncrypt_status(code: i32) -> VaultError {
    match code {
        NTE_USER_CANCELLED | ERROR_CANCELLED | WINBIO_E_CANCELED => VaultError::BiometricCanceled,
        NTE_NO_KEY | NTE_BAD_KEYSET | NTE_BAD_DATA => VaultError::BiometricKeyInvalid,
        WINBIO_E_CRED_PROV_SECURITY_LOCKOUT => VaultError::BiometricLockout,
        WINBIO_E_NO_MATCH => VaultError::BiometricFailed,
        NTE_INVALID_PARAMETER
        | NTE_SILENT_CONTEXT
        | NTE_NOT_SUPPORTED
        | NTE_BAD_FLAGS
        | NTE_PERM
        | WINBIO_E_DISABLED
        | WINBIO_E_CRED_PROV_DISABLED => VaultError::BiometricUnavailable,
        _ => VaultError::BiometricFailed,
    }
}

pub(crate) fn production_keystore() -> Arc<dyn VaultKeyStore> {
    // 没有当前用户的 LocalAppData 时不把密文写到共享目录，设备槽保持不可用。
    match std::env::var_os("LOCALAPPDATA") {
        Some(base) => Arc::new(WindowsHelloKeyStore::new(
            Arc::new(RealHello),
            PathBuf::from(base)
                .join("com.allinone.desktop")
                .join("vault-hello")
                .join("v1"),
        )),
        None => Arc::new(crate::keystore::UnavailableKeyStore),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RsaPadding {
    Oaep,
    Pkcs1,
}

pub(crate) fn split_wrapped_key(wrapped: &[u8]) -> VaultResult<(RsaPadding, &[u8])> {
    if wrapped.len() == OAEP_LEN {
        Ok((RsaPadding::Oaep, wrapped))
    } else if wrapped.len() == OAEP_LEN + PKCS1_PREFIX.len() && wrapped.starts_with(PKCS1_PREFIX) {
        Ok((RsaPadding::Pkcs1, &wrapped[PKCS1_PREFIX.len()..]))
    } else {
        Err(VaultError::BiometricKeyInvalid)
    }
}

pub(crate) struct RealHello;

impl HelloBackend for RealHello {
    fn gate(&self) -> VaultResult<()> {
        let _apartment = WinRtApartment::initialize()?;
        ngc_rsa_available()?;
        hello_configured()
    }

    fn protect_key(&self, slot_id: &Uuid, dkek: &[u8; 32]) -> VaultResult<Vec<u8>> {
        self.gate()?;
        let provider = open_provider()?;
        let name = key_name(slot_id)?;
        delete_key_if_present(&provider, &name, false)?;
        let created = create_decrypt_key(&provider, &name)?;
        let probe = match silent_probe(&provider, &name) {
            Ok(result) => result,
            Err(error) => {
                revoke_incomplete(&provider, &name, created);
                return Err(error);
            }
        };
        if !gesture_gate_holds(probe) {
            revoke_incomplete(&provider, &name, created);
            tracing::warn!("vault NGC key remained usable without a Hello gesture; slot refused");
            return Err(VaultError::BiometricUnavailable);
        }
        let ciphertext = match encrypt_silent(&provider, &name, dkek) {
            Ok(ciphertext) => ciphertext,
            Err(error) => {
                revoke_incomplete(&provider, &name, created);
                return Err(error);
            }
        };
        // 句柄交给释放；密文已经生成，保留密钥。
        drop(created);
        if ciphertext.len() != OAEP_LEN {
            let _ = delete_key_if_present(&provider, &name, false);
            return Err(VaultError::BiometricUnavailable);
        }
        Ok([PKCS1_PREFIX.as_slice(), &ciphertext].concat())
    }

    fn unprotect_key(&self, slot_id: &Uuid, wrapped: &[u8]) -> VaultResult<Zeroizing<[u8; 32]>> {
        let (padding, ciphertext) = split_wrapped_key(wrapped)?;
        let provider = open_provider()?;
        let name = key_name(slot_id)?;
        let key = open_key(&provider, &name, false)?;
        set_parent_window(&key)?;
        if let Err(error) = require_private_use_policy(&key) {
            return Err(error);
        }
        if let Err(error) = require_fresh_gesture(&key) {
            return Err(error);
        }
        decrypt_rsa(&key, ciphertext, false, padding)
    }

    fn delete_key(&self, slot_id: &Uuid) -> VaultResult<()> {
        let provider = open_provider()?;
        delete_key_if_present(&provider, &key_name(slot_id)?, true)
    }
}

// Passport 的持久密钥名是用户/提供程序的路径，不是任意应用别名。
// 缺少 SID// 前缀会让 NCryptCreatePersistedKey 返回 NTE_INVALID_PARAMETER。
fn key_name(slot_id: &Uuid) -> VaultResult<String> {
    Ok(format_key_name(&current_user_sid()?, slot_id))
}

pub(crate) fn format_key_name(sid: &str, slot_id: &Uuid) -> String {
    format!("{sid}//com.allinone.desktop/vault/{slot_id}")
}

fn current_user_sid() -> VaultResult<String> {
    use windows::Win32::{
        Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL},
        Security::{
            Authorization::ConvertSidToStringSidW, GetTokenInformation, TokenUser, TOKEN_QUERY,
            TOKEN_USER,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
            .map_err(|_| VaultError::BiometricUnavailable)?;
        let result = (|| {
            let mut len = 0;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
            if len == 0 || len > 65_536 {
                return Err(VaultError::BiometricUnavailable);
            }
            // TOKEN_USER 包含指针，使用指针宽度对齐的缓冲，且持有到 SID 字符串转换完成。
            let mut buffer = vec![0usize; (len as usize).div_ceil(std::mem::size_of::<usize>())];
            GetTokenInformation(
                token,
                TokenUser,
                Some(buffer.as_mut_ptr().cast()),
                len,
                &mut len,
            )
            .map_err(|_| VaultError::BiometricUnavailable)?;
            let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
            let mut sid = windows::core::PWSTR::null();
            ConvertSidToStringSidW(user.User.Sid, &mut sid)
                .map_err(|_| VaultError::BiometricUnavailable)?;
            let result = sid
                .to_string()
                .map_err(|_| VaultError::BiometricUnavailable);
            let _ = LocalFree(Some(HLOCAL(sid.0.cast())));
            result
        })();
        let _ = CloseHandle(token);
        result
    }
}

static OWNER_WINDOW: AtomicIsize = AtomicIsize::new(0);

/// 只由桌面宿主设置现有 dashboard HWND。不创建额外窗口，不接受其他进程的 HWND。
pub fn set_owner_window(handle: isize) -> VaultResult<()> {
    validate_owner_window(handle)?;
    OWNER_WINDOW.store(handle, Ordering::SeqCst);
    Ok(())
}

fn validate_owner_window(handle: isize) -> VaultResult<()> {
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow},
    };
    let hwnd = HWND(handle as *mut core::ffi::c_void);
    let mut process_id = 0;
    unsafe {
        if handle == 0 || !IsWindow(Some(hwnd)).as_bool() {
            return Err(VaultError::BiometricUnavailable);
        }
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));
    }
    if process_id != std::process::id() {
        return Err(VaultError::BiometricUnavailable);
    }
    Ok(())
}

fn set_parent_window(key: &Key) -> VaultResult<()> {
    let handle = OWNER_WINDOW.load(Ordering::SeqCst);
    // 独立手工硬件测试允许系统无 owner 的认证界面；桌面 IPC 必须先绑定 dashboard。
    if handle == 0 {
        return Ok(());
    }
    validate_owner_window(handle)?;
    unsafe {
        windows::Win32::Security::Cryptography::NCryptSetProperty(
            windows::Win32::Security::Cryptography::NCRYPT_HANDLE(key.0 .0),
            windows::Win32::Security::Cryptography::NCRYPT_WINDOW_HANDLE_PROPERTY,
            &handle.to_ne_bytes(),
            windows::Win32::Security::Cryptography::NCRYPT_FLAGS(0),
        )
    }
    .map_err(|error| map_ncrypt_status(error.code().0))
}

// NGC 提供程序的兼容属性；不属于 ncrypt.h 的跨提供程序合同。
// 设置、读取或静默解密反证任一不满足，登记失败，不退回无认证的密钥读取。
const NGC_CACHE_TYPE: windows::core::PCWSTR = windows::core::w!("NgcCacheType");
const NGC_CACHE_TYPE_LEGACY: windows::core::PCWSTR = windows::core::w!("NgcCacheTypeProperty");
const AUTH_MANDATORY: u32 = 1;

fn set_auth_mandatory(key: &Key) -> VaultResult<()> {
    let object = windows::Win32::Security::Cryptography::NCRYPT_HANDLE(key.0 .0);
    set_dword(object, NGC_CACHE_TYPE, AUTH_MANDATORY)
        .or_else(|_| set_dword(object, NGC_CACHE_TYPE_LEGACY, AUTH_MANDATORY))
}

fn require_auth_mandatory(key: &Key) -> VaultResult<()> {
    let object = windows::Win32::Security::Cryptography::NCRYPT_HANDLE(key.0 .0);
    let value =
        get_dword(object, NGC_CACHE_TYPE).or_else(|_| get_dword(object, NGC_CACHE_TYPE_LEGACY))?;
    if value != AUTH_MANDATORY {
        return Err(VaultError::BiometricKeyInvalid);
    }
    Ok(())
}

fn ngc_rsa_available() -> VaultResult<()> {
    let provider = open_provider()?;
    let status = unsafe {
        windows::Win32::Security::Cryptography::NCryptIsAlgSupported(
            provider.0,
            windows::Win32::Security::Cryptography::BCRYPT_RSA_ALGORITHM,
            windows::Win32::Security::Cryptography::NCRYPT_SILENT_FLAG.0,
        )
    };
    status.map_err(|error| {
        tracing::warn!(code = error.code().0, "Passport KSP rejected RSA");
        VaultError::BiometricUnavailable
    })
}

fn hello_configured() -> VaultResult<()> {
    use windows::Security::Credentials::UI::{
        UserConsentVerifier, UserConsentVerifierAvailability,
    };
    let availability = UserConsentVerifier::CheckAvailabilityAsync()
        .map_err(|error| {
            tracing::warn!(
                code = error.code().0,
                "Windows Hello availability query failed"
            );
            VaultError::BiometricUnavailable
        })?
        .join()
        .map_err(|error| {
            tracing::warn!(
                code = error.code().0,
                "Windows Hello availability join failed"
            );
            VaultError::BiometricUnavailable
        })?;
    match availability {
        UserConsentVerifierAvailability::Available => Ok(()),
        UserConsentVerifierAvailability::NotConfiguredForUser => {
            Err(VaultError::BiometricNotEnrolled)
        }
        _ => Err(VaultError::BiometricUnavailable),
    }
}

struct WinRtApartment {
    owned: bool,
}

impl WinRtApartment {
    fn initialize() -> VaultResult<Self> {
        use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
        use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
        unsafe {
            match RoInitialize(RO_INIT_MULTITHREADED) {
                Ok(()) => Ok(Self { owned: true }),
                Err(error) if error.code() == RPC_E_CHANGED_MODE => Ok(Self { owned: false }),
                Err(error) => {
                    tracing::warn!(
                        code = error.code().0,
                        "WinRT apartment initialization failed"
                    );
                    Err(VaultError::BiometricUnavailable)
                }
            }
        }
    }
}

impl Drop for WinRtApartment {
    fn drop(&mut self) {
        if self.owned {
            unsafe { windows::Win32::System::WinRT::RoUninitialize() };
        }
    }
}

struct Provider(windows::Win32::Security::Cryptography::NCRYPT_PROV_HANDLE);

impl Drop for Provider {
    fn drop(&mut self) {
        free_handle(self.0 .0);
        self.0 .0 = 0;
    }
}

struct Key(windows::Win32::Security::Cryptography::NCRYPT_KEY_HANDLE);

impl Key {
    /// 成功时 `NCryptDeleteKey` 会释放句柄，不能再 `NCryptFreeObject`。
    /// 失败时官方只把“释放句柄”写成成功后的结果，所有权仍留在调用方，由 Drop 回收。
    fn delete(mut self, silent: bool) -> VaultResult<()> {
        let flags = if silent {
            windows::Win32::Security::Cryptography::NCRYPT_SILENT_FLAG.0
        } else {
            0
        };
        match unsafe { windows::Win32::Security::Cryptography::NCryptDeleteKey(self.0, flags) } {
            Ok(()) => {
                self.0 .0 = 0;
                Ok(())
            }
            Err(error) => {
                tracing::warn!(code = error.code().0, "NCryptDeleteKey failed");
                Err(map_ncrypt_status(error.code().0))
            }
        }
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        free_handle(self.0 .0);
        self.0 .0 = 0;
    }
}

fn free_handle(value: usize) {
    if value != 0 {
        unsafe {
            let _ = windows::Win32::Security::Cryptography::NCryptFreeObject(
                windows::Win32::Security::Cryptography::NCRYPT_HANDLE(value),
            );
        }
    }
}

fn open_provider() -> VaultResult<Provider> {
    use windows::Win32::Security::Cryptography::{
        NCryptOpenStorageProvider, MS_NGC_KEY_STORAGE_PROVIDER, NCRYPT_PROV_HANDLE,
    };
    let mut handle = NCRYPT_PROV_HANDLE::default();
    unsafe { NCryptOpenStorageProvider(&mut handle, MS_NGC_KEY_STORAGE_PROVIDER, 0) }.map_err(
        |error| {
            tracing::warn!(
                code = error.code().0,
                "Passport key storage provider unavailable"
            );
            VaultError::BiometricUnavailable
        },
    )?;
    Ok(Provider(handle))
}

fn open_key(provider: &Provider, name: &str, silent: bool) -> VaultResult<Key> {
    use windows::Win32::Security::Cryptography::{
        NCryptOpenKey, CERT_KEY_SPEC, NCRYPT_FLAGS, NCRYPT_KEY_HANDLE, NCRYPT_SILENT_FLAG,
    };
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut handle = NCRYPT_KEY_HANDLE::default();
    let flags = if silent {
        NCRYPT_SILENT_FLAG
    } else {
        NCRYPT_FLAGS(0)
    };
    let opened = unsafe {
        NCryptOpenKey(
            provider.0,
            &mut handle,
            windows::core::PCWSTR(wide.as_ptr()),
            CERT_KEY_SPEC(0),
            flags,
        )
    };
    if let Err(error) = opened {
        if !handle.is_invalid() {
            free_handle(handle.0);
        }
        tracing::warn!(code = error.code().0, "NCryptOpenKey failed");
        return Err(map_ncrypt_status(error.code().0));
    }
    Ok(Key(handle))
}

/// `allow_prompt` 只给用户主动撤销。失败回滚始终静默，避免登记失败后再弹出认证框。
fn delete_key_if_present(provider: &Provider, name: &str, allow_prompt: bool) -> VaultResult<()> {
    match open_key(provider, name, true) {
        Ok(key) => key.delete(!allow_prompt),
        Err(VaultError::BiometricKeyInvalid) => Ok(()),
        Err(_) if allow_prompt => match open_key(provider, name, false) {
            Ok(key) => key.delete(false),
            Err(VaultError::BiometricKeyInvalid) => Ok(()),
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    }
}

fn revoke_incomplete(provider: &Provider, name: &str, key: Key) {
    if key.delete(true).is_err() {
        let _ = delete_key_if_present(provider, name, false);
    }
}

fn create_decrypt_key(provider: &Provider, name: &str) -> VaultResult<Key> {
    let key = create_unfinalized_key(provider, name)?;
    let finalized = (|| {
        configure_new_key(&key)?;
        unsafe {
            windows::Win32::Security::Cryptography::NCryptFinalizeKey(
                key.0,
                windows::Win32::Security::Cryptography::NCRYPT_FLAGS(0),
            )
        }
        .map_err(|error| {
            tracing::warn!(code = error.code().0, "NCryptFinalizeKey failed");
            map_ncrypt_status(error.code().0)
        })
    })();
    if let Err(error) = finalized {
        revoke_incomplete(provider, name, key);
        return Err(error);
    }
    Ok(key)
}

fn create_unfinalized_key(provider: &Provider, name: &str) -> VaultResult<Key> {
    use windows::Win32::Security::Cryptography::{
        NCryptCreatePersistedKey, BCRYPT_RSA_ALGORITHM, CERT_KEY_SPEC, NCRYPT_FLAGS,
        NCRYPT_KEY_HANDLE,
    };
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut handle = NCRYPT_KEY_HANDLE::default();
    let created = unsafe {
        NCryptCreatePersistedKey(
            provider.0,
            &mut handle,
            BCRYPT_RSA_ALGORITHM,
            windows::core::PCWSTR(wide.as_ptr()),
            CERT_KEY_SPEC(0),
            NCRYPT_FLAGS(0),
        )
    };
    if let Err(error) = created {
        if !handle.is_invalid() {
            revoke_incomplete(provider, name, Key(handle));
        }
        tracing::warn!(code = error.code().0, "NCryptCreatePersistedKey failed");
        return Err(map_ncrypt_status(error.code().0));
    }
    Ok(Key(handle))
}

fn configure_new_key(key: &Key) -> VaultResult<()> {
    use windows::Win32::Security::Cryptography::{
        NCRYPT_ALLOW_DECRYPT_FLAG, NCRYPT_EXPORT_POLICY_PROPERTY, NCRYPT_KEY_USAGE_PROPERTY,
        NCRYPT_LENGTH_PROPERTY,
    };
    let object = windows::Win32::Security::Cryptography::NCRYPT_HANDLE(key.0 .0);
    set_dword(object, NCRYPT_LENGTH_PROPERTY, 2048)?;
    set_dword(object, NCRYPT_KEY_USAGE_PROPERTY, NCRYPT_ALLOW_DECRYPT_FLAG)?;
    set_dword(object, NCRYPT_EXPORT_POLICY_PROPERTY, 0)?;
    set_auth_mandatory(key)?;
    set_parent_window(key)?;
    set_prompt(object)
}

#[cfg(test)]
pub(crate) fn probe_scoped_key_creation() -> VaultResult<()> {
    let provider = open_provider()?;
    let name = key_name(&Uuid::new_v4())?;
    let key = create_unfinalized_key(&provider, &name)?;
    // 未 finalize 的句柄 drop 后丢弃；不留下持久条目，也不触发认证。
    configure_new_key(&key)
}

#[cfg(test)]
pub(crate) fn diagnose_native_padding() -> VaultResult<()> {
    use windows::Win32::Security::Cryptography::*;
    let provider = open_provider()?;
    let name = key_name(&Uuid::new_v4())?;
    let key = create_unfinalized_key(&provider, &name)?;
    // 只诊断公钥加密能力：没有 DKEK，也不设置 AUTH_MANDATORY；整个创建必须静默。
    // 该测试密钥绝不交给 WindowsHelloKeyStore，也不写持久信封。
    let object = NCRYPT_HANDLE(key.0 .0);
    set_dword(object, NCRYPT_LENGTH_PROPERTY, 2048)?;
    set_dword(object, NCRYPT_KEY_USAGE_PROPERTY, NCRYPT_ALLOW_DECRYPT_FLAG)?;
    if let Err(error) = unsafe { NCryptFinalizeKey(key.0, NCRYPT_SILENT_FLAG) } {
        eprintln!(
            "Silent diagnostic key finalization failed: {:#x}",
            error.code().0
        );
        let _ = key.delete(true);
        return Err(map_ncrypt_status(error.code().0));
    }
    let result = (|| {
        let oaep = encrypt_public_oaep(&key, &[0xB3; 32])?;
        require_fresh_gesture(&key)?;
        let oaep_decrypt = rsa_crypt(key.0, &oaep, true, true, RsaPadding::Oaep);
        eprintln!(
            "BCrypt OAEP-SHA256 + Passport silent decrypt: {:?}",
            oaep_decrypt.as_ref().err().map(|code| format!("{code:#x}"))
        );
        for (label, hash, label_bytes, flag) in [
            (
                "OAEP-SHA256-label",
                BCRYPT_SHA256_ALGORITHM,
                OAEP_LABEL,
                NCRYPT_PAD_OAEP_FLAG,
            ),
            (
                "OAEP-SHA256-empty",
                BCRYPT_SHA256_ALGORITHM,
                &[][..],
                NCRYPT_PAD_OAEP_FLAG,
            ),
            (
                "OAEP-SHA1-empty",
                BCRYPT_SHA1_ALGORITHM,
                &[][..],
                NCRYPT_PAD_OAEP_FLAG,
            ),
            (
                "PKCS1",
                BCRYPT_SHA256_ALGORITHM,
                &[][..],
                NCRYPT_PAD_PKCS1_FLAG,
            ),
        ] {
            let info = BCRYPT_OAEP_PADDING_INFO {
                pszAlgId: hash,
                pbLabel: if label_bytes.is_empty() {
                    std::ptr::null_mut()
                } else {
                    label_bytes.as_ptr() as *mut u8
                },
                cbLabel: label_bytes.len() as u32,
            };
            let padding = (flag == NCRYPT_PAD_OAEP_FLAG)
                .then_some((&info as *const BCRYPT_OAEP_PADDING_INFO).cast());
            for silent in [false, true] {
                let flags = NCRYPT_FLAGS(flag.0 | if silent { NCRYPT_SILENT_FLAG.0 } else { 0 });
                let mut size = 0;
                let sized = unsafe {
                    NCryptEncrypt(key.0, Some(&[0xB3; 32]), padding, None, &mut size, flags)
                };
                let mut encrypted = [0u8; OAEP_LEN];
                let actual = unsafe {
                    NCryptEncrypt(
                        key.0,
                        Some(&[0xB3; 32]),
                        padding,
                        Some(&mut encrypted),
                        &mut size,
                        flags,
                    )
                };
                eprintln!(
                    "{label} silent={silent}: size-query={:?}, encrypt={:?}",
                    sized.as_ref().err().map(|e| format!("{:#x}", e.code().0)),
                    actual.as_ref().err().map(|e| format!("{:#x}", e.code().0))
                );
                if actual.is_ok() {
                    require_fresh_gesture(&key)?;
                    let mut output = Zeroizing::new([0u8; OAEP_LEN]);
                    let status = unsafe {
                        NCryptDecrypt(
                            key.0,
                            Some(&encrypted),
                            padding,
                            Some(output.as_mut_slice()),
                            &mut size,
                            NCRYPT_FLAGS(flag.0 | NCRYPT_SILENT_FLAG.0),
                        )
                    };
                    eprintln!(
                        "{label} silent-decrypt={:?}",
                        status.as_ref().err().map(|e| format!("{:#x}", e.code().0))
                    );
                }
            }
        }
        Ok(())
    })();
    let cleanup = key.delete(true);
    result.and(cleanup)
}

fn silent_probe(provider: &Provider, name: &str) -> VaultResult<SilentDecrypt> {
    let mut canary = [0u8; 32];
    if getrandom::fill(&mut canary).is_err() {
        canary.zeroize();
        return Err(VaultError::RandomSourceFailed);
    }
    let encrypted = encrypt_silent(provider, name, &canary);
    canary.zeroize();
    let encrypted = encrypted?;
    let key = open_key(provider, name, true)?;
    require_private_use_policy(&key)?;
    require_fresh_gesture(&key)?;
    clear_pin_cache(&key);
    let classified = match rsa_crypt(key.0, &encrypted, true, true, RsaPadding::Pkcs1) {
        Ok(mut plaintext) => {
            plaintext.zeroize();
            classify_silent_hresult(None)
        }
        Err(code) => {
            let classified = classify_silent_hresult(Some(code));
            if !gesture_gate_holds(classified) {
                tracing::warn!(code, "silent NGC decrypt did not prove a Hello gesture");
            }
            classified
        }
    };
    Ok(classified)
}

fn encrypt_silent(provider: &Provider, name: &str, plaintext: &[u8]) -> VaultResult<Vec<u8>> {
    let key = open_key(provider, name, true)?;
    let output = rsa_crypt(key.0, plaintext, true, false, RsaPadding::Pkcs1).map_err(|code| {
        tracing::warn!(code, "silent NGC RSA encrypt failed");
        map_ncrypt_status(code)
    })?;
    if output.len() != OAEP_LEN {
        return Err(VaultError::BiometricUnavailable);
    }
    Ok(output.to_vec())
}

/// 只导出公钥，在标准 BCrypt RSA 实现中做 OAEP；不导出 Passport 私钥。
#[cfg(test)]
fn encrypt_public_oaep(key: &Key, plaintext: &[u8]) -> VaultResult<Vec<u8>> {
    use windows::Win32::Security::Cryptography::*;
    let mut length = 0;
    unsafe {
        NCryptExportKey(
            key.0,
            None,
            BCRYPT_RSAPUBLIC_BLOB,
            None,
            None,
            &mut length,
            NCRYPT_SILENT_FLAG,
        )
    }
    .map_err(|error| map_ncrypt_status(error.code().0))?;
    if length == 0 || length > 4096 {
        return Err(VaultError::BiometricKeyInvalid);
    }
    let mut blob = vec![0u8; length as usize];
    unsafe {
        NCryptExportKey(
            key.0,
            None,
            BCRYPT_RSAPUBLIC_BLOB,
            None,
            Some(&mut blob),
            &mut length,
            NCRYPT_SILENT_FLAG,
        )
    }
    .map_err(|error| map_ncrypt_status(error.code().0))?;
    if length as usize > blob.len() {
        return Err(VaultError::BiometricKeyInvalid);
    }
    blob.truncate(length as usize);
    struct Algorithm(BCRYPT_ALG_HANDLE);
    impl Drop for Algorithm {
        fn drop(&mut self) {
            unsafe {
                let _ = BCryptCloseAlgorithmProvider(self.0, 0);
            }
        }
    }
    struct PublicKey(BCRYPT_KEY_HANDLE);
    impl Drop for PublicKey {
        fn drop(&mut self) {
            unsafe {
                let _ = BCryptDestroyKey(self.0);
            }
        }
    }
    let mut algorithm = BCRYPT_ALG_HANDLE::default();
    unsafe {
        BCryptOpenAlgorithmProvider(
            &mut algorithm,
            BCRYPT_RSA_ALGORITHM,
            None,
            BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS(0),
        )
    }
    .ok()
    .map_err(|_| VaultError::BiometricUnavailable)?;
    let algorithm = Algorithm(algorithm);
    let mut public_key = BCRYPT_KEY_HANDLE::default();
    unsafe {
        BCryptImportKeyPair(
            algorithm.0,
            None,
            BCRYPT_RSAPUBLIC_BLOB,
            &mut public_key,
            &blob,
            0,
        )
    }
    .ok()
    .map_err(|_| VaultError::BiometricKeyInvalid)?;
    let public_key = PublicKey(public_key);
    let info = BCRYPT_OAEP_PADDING_INFO {
        pszAlgId: BCRYPT_SHA256_ALGORITHM,
        pbLabel: OAEP_LABEL.as_ptr() as *mut u8,
        cbLabel: OAEP_LABEL.len() as u32,
    };
    let mut output = vec![0u8; OAEP_LEN];
    let mut written = 0;
    unsafe {
        BCryptEncrypt(
            public_key.0,
            Some(plaintext),
            Some((&info as *const BCRYPT_OAEP_PADDING_INFO).cast()),
            None,
            Some(&mut output),
            &mut written,
            BCRYPT_PAD_OAEP,
        )
    }
    .ok()
    .map_err(|_| VaultError::BiometricUnavailable)?;
    if written as usize != OAEP_LEN {
        return Err(VaultError::BiometricKeyInvalid);
    }
    Ok(output)
}

fn decrypt_rsa(
    key: &Key,
    wrapped: &[u8],
    silent: bool,
    padding: RsaPadding,
) -> VaultResult<Zeroizing<[u8; 32]>> {
    let output = rsa_crypt(key.0, wrapped, silent, true, padding).map_err(map_ncrypt_status)?;
    if output.len() != 32 {
        return Err(VaultError::BiometricKeyInvalid);
    }
    let mut dkek = Zeroizing::new([0u8; 32]);
    dkek.copy_from_slice(&output);
    Ok(dkek)
}

fn rsa_crypt(
    key: windows::Win32::Security::Cryptography::NCRYPT_KEY_HANDLE,
    input: &[u8],
    silent: bool,
    decrypting: bool,
    padding_kind: RsaPadding,
) -> Result<Zeroizing<Vec<u8>>, i32> {
    use windows::Win32::Security::Cryptography::{
        NCryptDecrypt, NCryptEncrypt, BCRYPT_OAEP_PADDING_INFO, BCRYPT_SHA256_ALGORITHM,
        NCRYPT_FLAGS, NCRYPT_PAD_OAEP_FLAG, NCRYPT_PAD_PKCS1_FLAG, NCRYPT_SILENT_FLAG,
    };
    let info = BCRYPT_OAEP_PADDING_INFO {
        pszAlgId: BCRYPT_SHA256_ALGORITHM,
        pbLabel: OAEP_LABEL.as_ptr() as *mut u8,
        cbLabel: OAEP_LABEL.len() as u32,
    };
    let mut flags = match padding_kind {
        RsaPadding::Oaep => NCRYPT_PAD_OAEP_FLAG.0,
        RsaPadding::Pkcs1 => NCRYPT_PAD_PKCS1_FLAG.0,
    };
    if silent {
        flags |= NCRYPT_SILENT_FLAG.0;
    }
    let flags = NCRYPT_FLAGS(flags);
    let padding = (padding_kind == RsaPadding::Oaep)
        .then_some((&info as *const BCRYPT_OAEP_PADDING_INFO).cast());
    // RSA-2048 密文和解密缓冲上界都是模长。一次调用避免查询长度时多一次认证。
    let size = OAEP_LEN as u32;
    if size == 0 || size > 4096 {
        return Err(NTE_BAD_DATA);
    }
    let mut output = Zeroizing::new(vec![0u8; size as usize]);
    let mut written = 0u32;
    let status = if decrypting {
        unsafe {
            NCryptDecrypt(
                key,
                Some(input),
                padding,
                Some(output.as_mut_slice()),
                &mut written,
                flags,
            )
        }
    } else {
        unsafe {
            NCryptEncrypt(
                key,
                Some(input),
                padding,
                Some(output.as_mut_slice()),
                &mut written,
                flags,
            )
        }
    };
    if let Err(error) = status {
        // 原生调用可能已经写入部分明文，整段缓冲都要清掉。
        output.zeroize();
        return Err(error.code().0);
    }
    let written = written as usize;
    if written > output.len() {
        output.zeroize();
        return Err(NTE_BAD_DATA);
    }
    // truncate 只缩短长度，容量里的尾部不会随 Drop 清零，先主动擦掉。
    output[written..].zeroize();
    output.truncate(written);
    Ok(output)
}

fn require_private_use_policy(key: &Key) -> VaultResult<()> {
    use windows::Win32::Security::Cryptography::{
        NCRYPT_ALLOW_ARCHIVING_FLAG, NCRYPT_ALLOW_DECRYPT_FLAG, NCRYPT_ALLOW_EXPORT_FLAG,
        NCRYPT_ALLOW_KEY_IMPORT_FLAG, NCRYPT_ALLOW_PLAINTEXT_ARCHIVING_FLAG,
        NCRYPT_ALLOW_PLAINTEXT_EXPORT_FLAG, NCRYPT_EXPORT_POLICY_PROPERTY,
        NCRYPT_KEY_USAGE_PROPERTY,
    };
    let object = windows::Win32::Security::Cryptography::NCRYPT_HANDLE(key.0 .0);
    require_auth_mandatory(key)?;
    let usage = get_dword(object, NCRYPT_KEY_USAGE_PROPERTY)?;
    // 读不到官方 Export Policy 就不能证明私钥不可导出。缺失不是“策略为 0”。
    let export_policy = get_dword(object, NCRYPT_EXPORT_POLICY_PROPERTY)?;
    let export_forbidden = NCRYPT_ALLOW_EXPORT_FLAG
        | NCRYPT_ALLOW_PLAINTEXT_EXPORT_FLAG
        | NCRYPT_ALLOW_ARCHIVING_FLAG
        | NCRYPT_ALLOW_PLAINTEXT_ARCHIVING_FLAG;
    if usage != NCRYPT_ALLOW_DECRYPT_FLAG
        || usage & NCRYPT_ALLOW_KEY_IMPORT_FLAG != 0
        || export_policy & export_forbidden != 0
    {
        tracing::warn!(usage, export_policy, "NGC key policy is not decrypt-only");
        return Err(VaultError::BiometricKeyInvalid);
    }
    Ok(())
}

fn require_fresh_gesture(key: &Key) -> VaultResult<()> {
    use windows::Win32::Security::Cryptography::{
        NCRYPT_PIN_CACHE_IS_GESTURE_REQUIRED_PROPERTY, NCRYPT_PIN_CACHE_REQUIRE_GESTURE_FLAG,
    };
    let object = windows::Win32::Security::Cryptography::NCRYPT_HANDLE(key.0 .0);
    set_dword(
        object,
        NCRYPT_PIN_CACHE_IS_GESTURE_REQUIRED_PROPERTY,
        NCRYPT_PIN_CACHE_REQUIRE_GESTURE_FLAG,
    )
    .map_err(|_| VaultError::BiometricUnavailable)
}

fn clear_pin_cache(key: &Key) {
    use windows::Win32::Security::Cryptography::{
        NCRYPT_PIN_CACHE_CLEAR_FOR_CALLING_PROCESS_OPTION, NCRYPT_PIN_CACHE_CLEAR_PROPERTY,
    };
    let object = windows::Win32::Security::Cryptography::NCRYPT_HANDLE(key.0 .0);
    if set_dword(
        object,
        NCRYPT_PIN_CACHE_CLEAR_PROPERTY,
        NCRYPT_PIN_CACHE_CLEAR_FOR_CALLING_PROCESS_OPTION,
    )
    .is_err()
    {
        tracing::warn!("pin cache clear was not accepted");
    }
}

fn set_dword(
    handle: windows::Win32::Security::Cryptography::NCRYPT_HANDLE,
    property: windows::core::PCWSTR,
    value: u32,
) -> VaultResult<()> {
    let bytes = value.to_le_bytes();
    unsafe {
        windows::Win32::Security::Cryptography::NCryptSetProperty(
            handle,
            property,
            &bytes,
            windows::Win32::Security::Cryptography::NCRYPT_FLAGS(0),
        )
    }
    .map_err(|error| {
        tracing::warn!(code = error.code().0, property = ?unsafe { property.to_string() }, "NCryptSetProperty failed");
        map_ncrypt_status(error.code().0)
    })
}

fn get_dword(
    handle: windows::Win32::Security::Cryptography::NCRYPT_HANDLE,
    property: windows::core::PCWSTR,
) -> VaultResult<u32> {
    let mut bytes = [0u8; 4];
    let mut written = 0u32;
    unsafe {
        windows::Win32::Security::Cryptography::NCryptGetProperty(
            handle,
            property,
            Some(&mut bytes),
            &mut written,
            windows::Win32::Security::OBJECT_SECURITY_INFORMATION(0),
        )
    }
    .map_err(|error| {
        tracing::warn!(code = error.code().0, property = ?unsafe { property.to_string() }, "NCryptGetProperty failed");
        map_ncrypt_status(error.code().0)
    })?;
    if written != 4 {
        return Err(VaultError::BiometricKeyInvalid);
    }
    Ok(u32::from_le_bytes(bytes))
}

fn set_prompt(handle: windows::Win32::Security::Cryptography::NCRYPT_HANDLE) -> VaultResult<()> {
    let mut wide: Vec<u16> = PROMPT.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes: Vec<u8> = wide.iter().flat_map(|unit| unit.to_le_bytes()).collect();
    let result = unsafe {
        windows::Win32::Security::Cryptography::NCryptSetProperty(
            handle,
            windows::Win32::Security::Cryptography::NCRYPT_USE_CONTEXT_PROPERTY,
            &bytes,
            windows::Win32::Security::Cryptography::NCRYPT_FLAGS(0),
        )
    };
    wide.zeroize();
    result.map_err(|error| {
        tracing::warn!(code = error.code().0, "Hello prompt property was rejected");
        map_ncrypt_status(error.code().0)
    })
}
