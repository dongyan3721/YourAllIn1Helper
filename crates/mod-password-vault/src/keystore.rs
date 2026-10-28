//! 设备密钥槽。生产环境只使用系统钥匙串的生物识别访问控制。
//! 内存实现只能由测试显式注入，`VaultService::new` 不会选中它。
use crate::{error::VaultError, VaultResult};
#[cfg(test)]
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;
use zeroize::Zeroizing;

pub(crate) const MACOS_BIOMETRY_BACKEND: &str = "macos_keychain_biometry_current_set_v1";
pub(crate) const MACOS_BIOMETRY_BACKEND_ID: u8 = 1;
const KEYCHAIN_SERVICE: &str = "com.allinone.desktop.vault.dkek.v1";

pub(crate) trait VaultKeyStore: Send + Sync {
    fn backend_id(&self) -> &'static str;
    fn backend_byte(&self) -> u8;
    /// 当前能否弹出 Touch ID。锁定或未登记时为 false，调用方改走主密码。
    fn available(&self) -> bool;
    fn gate(&self) -> VaultResult<()>;
    fn protect(&self, slot_id: &Uuid, dkek: &[u8; 32]) -> VaultResult<()>;
    fn unprotect(&self, slot_id: &Uuid) -> VaultResult<Zeroizing<[u8; 32]>>;
    fn revoke(&self, slot_id: &Uuid) -> VaultResult<()>;
}

pub(crate) fn production_keystore() -> Arc<dyn VaultKeyStore> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(MacosBiometryKeyStore)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Arc::new(UnavailableKeyStore)
    }
}

/// 测试与非 macOS 生产路径。protect 一律失败，不会写系统钥匙串。
pub(crate) struct UnavailableKeyStore;

impl VaultKeyStore for UnavailableKeyStore {
    fn backend_id(&self) -> &'static str {
        "unavailable"
    }

    fn backend_byte(&self) -> u8 {
        0
    }

    fn available(&self) -> bool {
        false
    }

    fn gate(&self) -> VaultResult<()> {
        Err(VaultError::BiometricUnavailable)
    }

    fn protect(&self, _slot_id: &Uuid, _dkek: &[u8; 32]) -> VaultResult<()> {
        Err(VaultError::BiometricUnavailable)
    }

    fn unprotect(&self, _slot_id: &Uuid) -> VaultResult<Zeroizing<[u8; 32]>> {
        Err(VaultError::BiometricUnavailable)
    }

    fn revoke(&self, _slot_id: &Uuid) -> VaultResult<()> {
        Ok(())
    }
}

/// 显式测试替身：DKEK 只活在进程内存里，用来验证包装协议，不代表 Touch ID。
#[cfg(test)]
pub(crate) struct MemoryKeyStore {
    items: parking_lot::Mutex<HashMap<Uuid, Zeroizing<[u8; 32]>>>,
}

#[cfg(test)]
impl MemoryKeyStore {
    pub(crate) fn new() -> Self {
        Self {
            items: parking_lot::Mutex::new(HashMap::new()),
        }
    }
}

#[cfg(test)]
impl VaultKeyStore for MemoryKeyStore {
    fn backend_id(&self) -> &'static str {
        MACOS_BIOMETRY_BACKEND
    }

    fn backend_byte(&self) -> u8 {
        MACOS_BIOMETRY_BACKEND_ID
    }

    fn available(&self) -> bool {
        true
    }

    fn gate(&self) -> VaultResult<()> {
        Ok(())
    }

    fn protect(&self, slot_id: &Uuid, dkek: &[u8; 32]) -> VaultResult<()> {
        self.items.lock().insert(*slot_id, Zeroizing::new(*dkek));
        Ok(())
    }

    fn unprotect(&self, slot_id: &Uuid) -> VaultResult<Zeroizing<[u8; 32]>> {
        self.items
            .lock()
            .get(slot_id)
            .cloned()
            .ok_or(VaultError::BiometricKeyInvalid)
    }

    fn revoke(&self, slot_id: &Uuid) -> VaultResult<()> {
        self.items.lock().remove(slot_id);
        Ok(())
    }
}

#[cfg(target_os = "macos")]
struct MacosBiometryKeyStore;

#[cfg(target_os = "macos")]
impl VaultKeyStore for MacosBiometryKeyStore {
    fn backend_id(&self) -> &'static str {
        MACOS_BIOMETRY_BACKEND
    }

    fn backend_byte(&self) -> u8 {
        MACOS_BIOMETRY_BACKEND_ID
    }

    fn available(&self) -> bool {
        matches!(os_hal::touch_id_probe(), os_hal::TouchIdProbe::Ready)
    }

    fn gate(&self) -> VaultResult<()> {
        match os_hal::touch_id_probe() {
            os_hal::TouchIdProbe::Ready => Ok(()),
            os_hal::TouchIdProbe::NotEnrolled => Err(VaultError::BiometricNotEnrolled),
            os_hal::TouchIdProbe::Lockout => Err(VaultError::BiometricLockout),
            os_hal::TouchIdProbe::Unavailable => Err(VaultError::BiometricUnavailable),
        }
    }

    fn protect(&self, slot_id: &Uuid, dkek: &[u8; 32]) -> VaultResult<()> {
        self.gate()?;
        // 同一个新 slot 上的残留项不能留着无访问控制的旧值。
        self.revoke(slot_id)?;
        let mut options = base_options(&slot_id.to_string());
        let access = security_framework::access_control::SecAccessControl::create_with_protection(
            Some(
                security_framework::access_control::ProtectionMode::AccessibleWhenUnlockedThisDeviceOnly,
            ),
            security_framework::passwords::AccessControlOptions::BIOMETRY_CURRENT_SET.bits(),
        )
        .map_err(map_keychain)?;
        options.set_access_control(access);
        security_framework::passwords::set_generic_password_options(dkek, options)
            .map_err(map_keychain)
    }

    fn unprotect(&self, slot_id: &Uuid) -> VaultResult<Zeroizing<[u8; 32]>> {
        self.gate()?;
        let mut bytes =
            security_framework::passwords::generic_password(base_options(&slot_id.to_string()))
                .map_err(map_keychain)?;
        if bytes.len() != 32 {
            bytes.zeroize();
            return Err(VaultError::BiometricKeyInvalid);
        }
        let mut dkek = Zeroizing::new([0u8; 32]);
        dkek.copy_from_slice(&bytes);
        bytes.zeroize();
        Ok(dkek)
    }

    fn revoke(&self, slot_id: &Uuid) -> VaultResult<()> {
        match security_framework::passwords::delete_generic_password_options(base_options(
            &slot_id.to_string(),
        )) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
            Err(error) => Err(map_keychain(error)),
        }
    }
}

#[cfg(target_os = "macos")]
fn base_options(account: &str) -> security_framework::passwords::PasswordOptions {
    let mut options = security_framework::passwords::PasswordOptions::new_generic_password(
        KEYCHAIN_SERVICE,
        account,
    );
    options.set_access_synchronized(Some(false));
    options.use_protected_keychain();
    options
}

#[cfg(target_os = "macos")]
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

#[cfg(target_os = "macos")]
fn map_keychain(error: security_framework::base::Error) -> VaultError {
    let code = error.code();
    tracing::warn!(code, "vault device keychain operation failed");
    match code {
        -128 | -2 => VaultError::BiometricCanceled,
        -25293 | -1 => VaultError::BiometricFailed,
        -8 => VaultError::BiometricLockout,
        -7 => VaultError::BiometricNotEnrolled,
        -6 | -25308 => VaultError::BiometricUnavailable,
        // errSecMissingEntitlement：数据保护钥匙串拒绝没有授权访问组的签名。
        -34018 => VaultError::BiometricKeychainDenied,
        ERR_SEC_ITEM_NOT_FOUND => VaultError::BiometricKeyInvalid,
        _ => VaultError::BiometricFailed,
    }
}

#[cfg(target_os = "macos")]
trait ZeroizeBytes {
    fn zeroize(&mut self);
}

#[cfg(target_os = "macos")]
impl ZeroizeBytes for Vec<u8> {
    fn zeroize(&mut self) {
        zeroize::Zeroize::zeroize(self.as_mut_slice());
        self.clear();
    }
}
