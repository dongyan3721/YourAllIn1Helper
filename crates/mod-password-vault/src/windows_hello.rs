//! Windows Hello 设备密钥。
//!
//! 安全边界（以 Windows SDK 10.0.26100 的 `ncrypt.h` 为准）：
//! - DKEK 用 Microsoft Passport Key Storage Provider 中不可导出的 RSA-2048 公钥做 OAEP-SHA256 包装。
//! - 解开 DKEK 的唯一路径是 `NCryptDecrypt`。调用前设置官方属性
//!   `NCRYPT_PIN_CACHE_IS_GESTURE_REQUIRED_PROPERTY`，并带上 `NCRYPT_NO_CACHED_PASSWORD`。
//! - 登记时先用 `NCRYPT_SILENT_FLAG` 解密一段随机探测值。只有返回 `NTE_SILENT_CONTEXT`
//!   才说明这次私钥操作必须弹出系统界面。静默解密一旦成功，立即删除密钥并报告不可用。
//! - `UserConsentVerifier.CheckAvailabilityAsync` 只用来判断 Hello 是否已配置，不参与释放密钥。
//!   不调用同意验证器的验证请求，也不把验证成功当成读取 DPAPI 或凭据管理器的条件。
//! - Windows Hello 手势包含人脸、指纹或 PIN。SDK 没有等价于 macOS `biometryCurrentSet`
//!   的“拒绝 PIN、生物识别集合变化后失效”标志，因此本后端不得宣称生物识别等价。
//! - 提供程序、算法、策略或静默探测不满足上述边界时返回不可用，主密码继续有效。
use crate::{
    error::{VaultError, VaultResult},
    hello_store::{HelloBackend, WindowsHelloKeyStore, OAEP_LEN},
    keystore::VaultKeyStore,
};
use std::{path::PathBuf, sync::Arc};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

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
        NTE_SILENT_CONTEXT | NTE_NOT_SUPPORTED | NTE_BAD_FLAGS | NTE_PERM | WINBIO_E_DISABLED
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
        let name = key_name(slot_id);
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
        Ok(ciphertext)
    }

    fn unprotect_key(&self, slot_id: &Uuid, wrapped: &[u8]) -> VaultResult<Zeroizing<[u8; 32]>> {
        if wrapped.len() != OAEP_LEN {
            return Err(VaultError::BiometricKeyInvalid);
        }
        let provider = open_provider()?;
        let name = key_name(slot_id);
        let key = open_key(&provider, &name, false)?;
        if let Err(error) = require_private_use_policy(&key) {
            return Err(error);
        }
        if let Err(error) = require_fresh_gesture(&key) {
            return Err(error);
        }
        decrypt_oaep(&key, wrapped, false)
    }

    fn delete_key(&self, slot_id: &Uuid) -> VaultResult<()> {
        let provider = open_provider()?;
        delete_key_if_present(&provider, &key_name(slot_id), true)
    }
}

fn key_name(slot_id: &Uuid) -> String {
    format!("aio.vault.dkek.{slot_id}")
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
    use windows::Security::Credentials::UI::{UserConsentVerifier, UserConsentVerifierAvailability};
    let availability = UserConsentVerifier::CheckAvailabilityAsync()
        .map_err(|error| {
            tracing::warn!(code = error.code().0, "Windows Hello availability query failed");
            VaultError::BiometricUnavailable
        })?
        .join()
        .map_err(|error| {
            tracing::warn!(code = error.code().0, "Windows Hello availability join failed");
            VaultError::BiometricUnavailable
        })?;
    match availability {
        UserConsentVerifierAvailability::Available => Ok(()),
        UserConsentVerifierAvailability::NotConfiguredForUser => Err(VaultError::BiometricNotEnrolled),
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
                    tracing::warn!(code = error.code().0, "WinRT apartment initialization failed");
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
    fn delete(mut self) -> VaultResult<()> {
        match unsafe { windows::Win32::Security::Cryptography::NCryptDeleteKey(self.0, 0) } {
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
        NCryptOpenStorageProvider, NCRYPT_PROV_HANDLE, MS_NGC_KEY_STORAGE_PROVIDER,
    };
    let mut handle = NCRYPT_PROV_HANDLE::default();
    unsafe { NCryptOpenStorageProvider(&mut handle, MS_NGC_KEY_STORAGE_PROVIDER, 0) }.map_err(
        |error| {
            tracing::warn!(code = error.code().0, "Passport key storage provider unavailable");
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
        Ok(key) => key.delete(),
        Err(VaultError::BiometricKeyInvalid) => Ok(()),
        Err(_) if allow_prompt => match open_key(provider, name, false) {
            Ok(key) => key.delete(),
            Err(VaultError::BiometricKeyInvalid) => Ok(()),
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    }
}

fn revoke_incomplete(provider: &Provider, name: &str, key: Key) {
    if key.delete().is_err() {
        let _ = delete_key_if_present(provider, name, false);
    }
}

fn create_decrypt_key(provider: &Provider, name: &str) -> VaultResult<Key> {
    use windows::Win32::Security::Cryptography::{
        NCryptCreatePersistedKey, NCryptFinalizeKey, BCRYPT_RSA_ALGORITHM, CERT_KEY_SPEC,
        NCRYPT_ALLOW_DECRYPT_FLAG, NCRYPT_EXPORT_POLICY_PROPERTY, NCRYPT_FLAGS, NCRYPT_KEY_HANDLE,
        NCRYPT_KEY_USAGE_PROPERTY, NCRYPT_LENGTH_PROPERTY, NCRYPT_OVERWRITE_KEY_FLAG,
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
            NCRYPT_OVERWRITE_KEY_FLAG,
        )
    };
    if let Err(error) = created {
        if !handle.is_invalid() {
            revoke_incomplete(provider, name, Key(handle));
        }
        tracing::warn!(code = error.code().0, "NCryptCreatePersistedKey failed");
        return Err(map_ncrypt_status(error.code().0));
    }
    let key = Key(handle);
    let object = windows::Win32::Security::Cryptography::NCRYPT_HANDLE(key.0 .0);
    let finalized = (|| {
        set_dword(object, NCRYPT_LENGTH_PROPERTY, 2048)?;
        set_dword(object, NCRYPT_KEY_USAGE_PROPERTY, NCRYPT_ALLOW_DECRYPT_FLAG)?;
        set_dword(object, NCRYPT_EXPORT_POLICY_PROPERTY, 0)?;
        set_prompt(object)?;
        unsafe { NCryptFinalizeKey(key.0, NCRYPT_FLAGS(0)) }.map_err(|error| {
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
    let classified = match rsa_oaep(key.0, &encrypted, true, true) {
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
    let output = rsa_oaep(key.0, plaintext, true, false).map_err(|code| {
        tracing::warn!(code, "silent NGC encrypt failed");
        map_ncrypt_status(code)
    })?;
    if output.len() != OAEP_LEN {
        return Err(VaultError::BiometricUnavailable);
    }
    Ok(output.to_vec())
}

fn decrypt_oaep(
    key: &Key,
    wrapped: &[u8],
    silent: bool,
) -> VaultResult<Zeroizing<[u8; 32]>> {
    let output = rsa_oaep(key.0, wrapped, silent, true).map_err(map_ncrypt_status)?;
    if output.len() != 32 {
        return Err(VaultError::BiometricKeyInvalid);
    }
    let mut dkek = Zeroizing::new([0u8; 32]);
    dkek.copy_from_slice(&output);
    Ok(dkek)
}

fn rsa_oaep(
    key: windows::Win32::Security::Cryptography::NCRYPT_KEY_HANDLE,
    input: &[u8],
    silent: bool,
    decrypting: bool,
) -> Result<Zeroizing<Vec<u8>>, i32> {
    use windows::Win32::Security::Cryptography::{
        NCryptDecrypt, NCryptEncrypt, BCRYPT_OAEP_PADDING_INFO, BCRYPT_SHA256_ALGORITHM,
        NCRYPT_FLAGS, NCRYPT_NO_CACHED_PASSWORD, NCRYPT_PAD_OAEP_FLAG, NCRYPT_SILENT_FLAG,
    };
    let info = BCRYPT_OAEP_PADDING_INFO {
        pszAlgId: BCRYPT_SHA256_ALGORITHM,
        pbLabel: OAEP_LABEL.as_ptr() as *mut u8,
        cbLabel: OAEP_LABEL.len() as u32,
    };
    let mut flags = NCRYPT_PAD_OAEP_FLAG.0;
    if decrypting {
        flags |= NCRYPT_NO_CACHED_PASSWORD;
    }
    if silent {
        flags |= NCRYPT_SILENT_FLAG.0;
    }
    let flags = NCRYPT_FLAGS(flags);
    let mut size = 0u32;
    let padding = &info as *const BCRYPT_OAEP_PADDING_INFO as *const core::ffi::c_void;
    let status = if decrypting {
        unsafe { NCryptDecrypt(key, Some(input), Some(padding), None, &mut size, flags) }
    } else {
        unsafe { NCryptEncrypt(key, Some(input), Some(padding), None, &mut size, flags) }
    };
    status.map_err(|error| error.code().0)?;
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
                Some(padding),
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
                Some(padding),
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
        NCRYPT_ALLOW_PLAINTEXT_EXPORT_FLAG, NCRYPT_EXPORT_POLICY_PROPERTY, NCRYPT_KEY_USAGE_PROPERTY,
    };
    let object = windows::Win32::Security::Cryptography::NCRYPT_HANDLE(key.0 .0);
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
        tracing::warn!(code = error.code().0, "NCryptSetProperty failed");
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
        tracing::warn!(code = error.code().0, "NCryptGetProperty failed");
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
