use crate::{
    error::{VaultError, VaultResult},
    model::FieldId,
};
use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Nonce,
};
use argon2::{Algorithm, Argon2, Params, Version};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

pub(crate) const CRYPTO_VERSION: u16 = 1;
pub(crate) const KDF_PROFILE: &str = "argon2id-v19-m65536-t3-p4";
pub(crate) const SALT_LEN: usize = 16;
pub(crate) const NONCE_LEN: usize = 12;
pub(crate) const TAG_LEN: usize = 16;
pub(crate) const KEY_LEN: usize = 32;

/// 生产唯一随机源；确定性或失败适配器只能在显式测试中注入。
pub trait VaultRandom: Send + Sync + 'static {
    fn fill(&self, destination: &mut [u8]) -> VaultResult<()>;
}

#[derive(Debug, Default)]
pub struct OsRandom;

impl VaultRandom for OsRandom {
    fn fill(&self, destination: &mut [u8]) -> VaultResult<()> {
        getrandom::fill(destination).map_err(|_| VaultError::RandomSourceFailed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KdfProfile {
    pub m_cost_kib: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

impl KdfProfile {
    pub(crate) const V1: Self = Self {
        m_cost_kib: 65_536,
        t_cost: 3,
        p_cost: 4,
    };

    /// v1 只接受已审查的命名 profile，拒绝由磁盘参数触发不受控内存分配。
    pub(crate) fn from_metadata(
        name: &str,
        m_cost_kib: u32,
        t_cost: u32,
        p_cost: u32,
    ) -> VaultResult<Self> {
        let profile = Self {
            m_cost_kib,
            t_cost,
            p_cost,
        };
        if name != KDF_PROFILE || profile != Self::V1 {
            return Err(VaultError::UnsupportedFormat);
        }
        Ok(profile)
    }

    pub(crate) fn params(self) -> VaultResult<Params> {
        Params::new(self.m_cost_kib, self.t_cost, self.p_cost, Some(KEY_LEN))
            .map_err(|_| VaultError::UnsupportedFormat)
    }
}

pub(crate) fn derive_kek(
    profile: KdfProfile,
    password: &[u8],
    salt: &[u8],
) -> VaultResult<Zeroizing<[u8; KEY_LEN]>> {
    if salt.len() != SALT_LEN {
        return Err(VaultError::UnsupportedFormat);
    }
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, profile.params()?);
    let mut kek = Zeroizing::new([0u8; KEY_LEN]);
    argon2
        .hash_password_into(password, salt, &mut *kek)
        .map_err(|_| VaultError::UnlockFailed)?;
    Ok(kek)
}

pub(crate) fn random_uuid(random: &dyn VaultRandom) -> VaultResult<Uuid> {
    let mut bytes = [0u8; 16];
    random.fill(&mut bytes)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(Uuid::from_bytes(bytes))
}

pub(crate) fn random_nonce(random: &dyn VaultRandom) -> VaultResult<[u8; NONCE_LEN]> {
    let mut nonce = [0u8; NONCE_LEN];
    random.fill(&mut nonce)?;
    Ok(nonce)
}

fn cipher(key: &[u8; KEY_LEN]) -> VaultResult<Aes256Gcm> {
    Aes256Gcm::new_from_slice(key).map_err(|_| VaultError::DataIntegrityError)
}

// 固定布局：7 + 2 + 16 + 16 + 1 + 8 = 50 字节。
fn field_aad_vec(
    vault_id: &Uuid,
    entry_id: &Uuid,
    field_id: FieldId,
    record_version: u64,
) -> Vec<u8> {
    let mut aad = Vec::with_capacity(50);
    aad.extend_from_slice(b"AIOVDF1");
    aad.extend_from_slice(&CRYPTO_VERSION.to_be_bytes());
    aad.extend_from_slice(vault_id.as_bytes());
    aad.extend_from_slice(entry_id.as_bytes());
    aad.push(field_id as u8);
    aad.extend_from_slice(&record_version.to_be_bytes());
    aad
}

fn master_aad(vault_id: &Uuid) -> Vec<u8> {
    let mut aad = Vec::with_capacity(7 + 2 + 16 + KDF_PROFILE.len());
    aad.extend_from_slice(b"AIOVMW1");
    aad.extend_from_slice(&CRYPTO_VERSION.to_be_bytes());
    aad.extend_from_slice(vault_id.as_bytes());
    aad.extend_from_slice(KDF_PROFILE.as_bytes());
    aad
}

pub(crate) fn encrypt_field(
    dek: &[u8; KEY_LEN],
    vault_id: &Uuid,
    entry_id: &Uuid,
    field_id: FieldId,
    record_version: u64,
    plaintext: &[u8],
    nonce: &[u8; NONCE_LEN],
) -> VaultResult<(Vec<u8>, [u8; TAG_LEN])> {
    let encrypted = cipher(dek)?
        .encrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: plaintext,
                aad: &field_aad_vec(vault_id, entry_id, field_id, record_version),
            },
        )
        .map_err(|_| VaultError::DataIntegrityError)?;
    if encrypted.len() != plaintext.len() + TAG_LEN {
        return Err(VaultError::DataIntegrityError);
    }
    let split = encrypted.len() - TAG_LEN;
    let mut tag = [0u8; TAG_LEN];
    tag.copy_from_slice(&encrypted[split..]);
    Ok((encrypted[..split].to_vec(), tag))
}

pub(crate) fn decrypt_field(
    dek: &[u8; KEY_LEN],
    vault_id: &Uuid,
    entry_id: &Uuid,
    field_id: FieldId,
    crypto_version: u16,
    record_version: u64,
    nonce: &[u8],
    ciphertext: &[u8],
    tag: &[u8],
) -> VaultResult<Zeroizing<Vec<u8>>> {
    if crypto_version != CRYPTO_VERSION
        || nonce.len() != NONCE_LEN
        || tag.len() != TAG_LEN
        || ciphertext.len() > crate::model::MAX_FIELD_BYTES
    {
        return Err(VaultError::DataIntegrityError);
    }
    let mut combined = Zeroizing::new(Vec::with_capacity(ciphertext.len() + TAG_LEN));
    combined.extend_from_slice(ciphertext);
    combined.extend_from_slice(tag);
    let plaintext = cipher(dek)?
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: combined.as_slice(),
                aad: &field_aad_vec(vault_id, entry_id, field_id, record_version),
            },
        )
        .map_err(|_| VaultError::DataIntegrityError)?;
    combined.zeroize();
    Ok(Zeroizing::new(plaintext))
}

pub(crate) fn wrap_dek(
    kek: &[u8; KEY_LEN],
    vault_id: &Uuid,
    dek: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
) -> VaultResult<[u8; KEY_LEN + TAG_LEN]> {
    let encrypted = cipher(kek)?
        .encrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: dek,
                aad: &master_aad(vault_id),
            },
        )
        .map_err(|_| VaultError::DataIntegrityError)?;
    let mut wrapped = [0u8; KEY_LEN + TAG_LEN];
    wrapped.copy_from_slice(&encrypted);
    Ok(wrapped)
}

pub(crate) fn unwrap_dek(
    kek: &[u8; KEY_LEN],
    vault_id: &Uuid,
    nonce: &[u8],
    wrapped: &[u8],
) -> VaultResult<Zeroizing<[u8; KEY_LEN]>> {
    if nonce.len() != NONCE_LEN || wrapped.len() != KEY_LEN + TAG_LEN {
        return Err(VaultError::UnlockFailed);
    }
    let plaintext = cipher(kek)
        .map_err(|_| VaultError::UnlockFailed)?
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: wrapped,
                aad: &master_aad(vault_id),
            },
        )
        .map_err(|_| VaultError::UnlockFailed)?;
    if plaintext.len() != KEY_LEN {
        return Err(VaultError::UnlockFailed);
    }
    let mut dek = Zeroizing::new([0u8; KEY_LEN]);
    dek.copy_from_slice(&plaintext);
    Ok(dek)
}

pub(crate) fn plaintext_to_string(mut plaintext: Zeroizing<Vec<u8>>) -> VaultResult<String> {
    // 将同一分配转移给 IPC 返回值；此后由前端短驻留契约管理，不宣称物理清零。
    String::from_utf8(std::mem::take(&mut *plaintext)).map_err(|_| VaultError::DataIntegrityError)
}
