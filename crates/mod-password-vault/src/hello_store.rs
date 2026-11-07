//! Windows Hello 设备密钥的本地信封。
//!
//! 文件里只保存 RSA-OAEP 密文，不保存 DKEK，也不经过 DPAPI 或凭据管理器。
//! 能否解开完全取决于 `HelloBackend::unprotect_key` 的私钥操作。
use crate::{
    error::{VaultError, VaultResult},
    keystore::{VaultKeyStore, WINDOWS_HELLO_BACKEND, WINDOWS_HELLO_BACKEND_ID},
    model::DeviceKeyKind,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

pub(crate) const OAEP_LEN: usize = 256;
const MAGIC: &[u8] = b"AIOHELLO";
const VERSION: u8 = 1;
const HEADER_LEN: usize = MAGIC.len() + 2;
pub(crate) const ENVELOPE_LEN: usize = HEADER_LEN + OAEP_LEN;

/// 真正执行 NGC 私钥操作的后端。测试可以替换它，生产实现只有 Windows Hello。
pub(crate) trait HelloBackend: Send + Sync {
    fn gate(&self) -> VaultResult<()>;
    fn protect_key(&self, slot_id: &Uuid, dkek: &[u8; 32]) -> VaultResult<Vec<u8>>;
    fn unprotect_key(&self, slot_id: &Uuid, wrapped: &[u8]) -> VaultResult<Zeroizing<[u8; 32]>>;
    fn delete_key(&self, slot_id: &Uuid) -> VaultResult<()>;
}

pub(crate) struct WindowsHelloKeyStore {
    backend: Arc<dyn HelloBackend>,
    root: PathBuf,
}

impl WindowsHelloKeyStore {
    pub(crate) fn new(backend: Arc<dyn HelloBackend>, root: PathBuf) -> Self {
        Self { backend, root }
    }

    fn path_for(&self, slot_id: &Uuid) -> PathBuf {
        // 只用规范 UUID 文件名，避免把槽标识拼成目录穿越。
        self.root.join(format!("{slot_id}.oaep"))
    }
}

impl VaultKeyStore for WindowsHelloKeyStore {
    fn backend_id(&self) -> &'static str {
        WINDOWS_HELLO_BACKEND
    }

    fn backend_byte(&self) -> u8 {
        WINDOWS_HELLO_BACKEND_ID
    }

    fn device_key_kind(&self) -> DeviceKeyKind {
        DeviceKeyKind::WindowsHelloGesture
    }

    fn available(&self) -> bool {
        self.backend.gate().is_ok()
    }

    fn gate(&self) -> VaultResult<()> {
        self.backend.gate()
    }

    fn protect(&self, slot_id: &Uuid, dkek: &[u8; 32]) -> VaultResult<()> {
        self.backend.gate()?;
        // 同一槽位先拆掉旧私钥和旧密文，不能留下可以无手势解开的残留。
        self.backend.delete_key(slot_id)?;
        let path = self.path_for(slot_id);
        discard_slot_files(&path)?;
        let mut wrapped = self.backend.protect_key(slot_id, dkek)?;
        if let Err(error) = write_envelope(&path, &wrapped) {
            wrapped.zeroize();
            let _ = self.backend.delete_key(slot_id);
            let _ = discard_slot_files(&path);
            return Err(error);
        }
        wrapped.zeroize();
        Ok(())
    }

    fn unprotect(&self, slot_id: &Uuid) -> VaultResult<Zeroizing<[u8; 32]>> {
        let wrapped = read_envelope(&self.path_for(slot_id))?;
        // 这里不读取“先前认证成功”的标志；没有私钥解密就没有 DKEK。
        self.backend.unprotect_key(slot_id, &wrapped)
    }

    fn revoke(&self, slot_id: &Uuid) -> VaultResult<()> {
        let key_result = self.backend.delete_key(slot_id);
        let file_result = discard_slot_files(&self.path_for(slot_id));
        key_result.and(file_result)
    }
}

pub(crate) fn parse_envelope(bytes: &[u8]) -> VaultResult<Zeroizing<Vec<u8>>> {
    if bytes.len() != ENVELOPE_LEN
        || bytes.len() == 32
        || &bytes[..MAGIC.len()] != MAGIC
        || bytes[MAGIC.len()] != VERSION
        || bytes[MAGIC.len() + 1] != 0
    {
        return Err(VaultError::BiometricKeyInvalid);
    }
    let ciphertext = &bytes[HEADER_LEN..];
    if ciphertext.len() != OAEP_LEN {
        return Err(VaultError::BiometricKeyInvalid);
    }
    Ok(Zeroizing::new(ciphertext.to_vec()))
}

fn write_envelope(path: &Path, ciphertext: &[u8]) -> VaultResult<()> {
    if ciphertext.len() != OAEP_LEN {
        return Err(VaultError::BiometricUnavailable);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| VaultError::BiometricUnavailable)?;
    }
    let mut bytes = Zeroizing::new(Vec::with_capacity(ENVELOPE_LEN));
    bytes.extend_from_slice(MAGIC);
    bytes.push(VERSION);
    bytes.push(0);
    bytes.extend_from_slice(ciphertext);
    let tmp = temp_path(path);
    let _ = fs::remove_file(&tmp);
    let write_result = fs::write(&tmp, bytes.as_slice());
    bytes.zeroize();
    if write_result.is_err() {
        let _ = fs::remove_file(&tmp);
        return Err(VaultError::BiometricUnavailable);
    }
    if path.exists() && fs::remove_file(path).is_err() {
        let _ = fs::remove_file(&tmp);
        return Err(VaultError::BiometricUnavailable);
    }
    if fs::rename(&tmp, path).is_err() {
        let _ = fs::remove_file(&tmp);
        return Err(VaultError::BiometricUnavailable);
    }
    Ok(())
}

fn temp_path(path: &Path) -> PathBuf {
    path.with_extension("tmp")
}

fn discard_slot_files(path: &Path) -> VaultResult<()> {
    let persisted = remove_if_present(path);
    let temporary = remove_if_present(&temp_path(path));
    persisted.and(temporary)
}

fn read_envelope(path: &Path) -> VaultResult<Zeroizing<Vec<u8>>> {
    let mut bytes = fs::read(path).map_err(|_| VaultError::BiometricKeyInvalid)?;
    let parsed = parse_envelope(&bytes);
    zeroize::Zeroize::zeroize(bytes.as_mut_slice());
    parsed
}

fn remove_if_present(path: &Path) -> VaultResult<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(VaultError::BiometricUnavailable),
    }
}
