use crate::{
    crypto::{KdfProfile, CRYPTO_VERSION, KDF_PROFILE, NONCE_LEN, SALT_LEN, TAG_LEN},
    error::{VaultError, VaultResult},
    model::{FieldId, MODULE_ID},
};
use app_core::ModuleMigration;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::collections::HashSet;
use uuid::Uuid;

const MIGRATION_V1_SQL: &str = "
CREATE TABLE vault_meta (
    id INTEGER PRIMARY KEY CHECK(id = 1),
    vault_id TEXT NOT NULL UNIQUE,
    crypto_version INTEGER NOT NULL CHECK(crypto_version = 1),
    kdf_profile TEXT NOT NULL,
    kdf_m_cost_kib INTEGER NOT NULL,
    kdf_t_cost INTEGER NOT NULL,
    kdf_p_cost INTEGER NOT NULL,
    kdf_salt BLOB NOT NULL CHECK(length(kdf_salt) = 16),
    master_wrap_nonce BLOB NOT NULL CHECK(length(master_wrap_nonce) = 12),
    master_wrapped_dek BLOB NOT NULL CHECK(length(master_wrapped_dek) = 48),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE password_vault_entries (
    id TEXT PRIMARY KEY,
    vault_id TEXT NOT NULL,
    record_version INTEGER NOT NULL CHECK(record_version >= 1),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    archived INTEGER NOT NULL DEFAULT 0 CHECK(archived IN (0, 1))
);
CREATE INDEX password_vault_entries_order
    ON password_vault_entries(updated_at DESC, id DESC);
CREATE TABLE password_vault_fields (
    entry_id TEXT NOT NULL REFERENCES password_vault_entries(id) ON DELETE CASCADE,
    field_id INTEGER NOT NULL CHECK(field_id BETWEEN 1 AND 7),
    crypto_version INTEGER NOT NULL CHECK(crypto_version = 1),
    record_version INTEGER NOT NULL CHECK(record_version >= 1),
    nonce BLOB NOT NULL CHECK(length(nonce) = 12),
    ciphertext BLOB NOT NULL,
    tag BLOB NOT NULL CHECK(length(tag) = 16),
    PRIMARY KEY(entry_id, field_id),
    UNIQUE(entry_id, nonce)
);
CREATE TABLE vault_device_slots (
    slot_id TEXT PRIMARY KEY,
    vault_id TEXT NOT NULL,
    backend TEXT NOT NULL,
    crypto_version INTEGER NOT NULL,
    device_wrap_nonce BLOB NOT NULL CHECK(length(device_wrap_nonce) = 12),
    device_wrapped_dek BLOB NOT NULL CHECK(length(device_wrapped_dek) = 48),
    os_protected_dkek BLOB,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
";

pub fn migrations() -> Vec<ModuleMigration> {
    vec![ModuleMigration::new(
        MODULE_ID,
        1,
        "Create empty v1 master-password vault tables without altering existing modules",
        MIGRATION_V1_SQL,
    )]
}

#[derive(Debug)]
pub(crate) struct MasterSlot {
    pub vault_id: Uuid,
    pub profile: KdfProfile,
    pub salt: Vec<u8>,
    pub wrap_nonce: Vec<u8>,
    pub wrapped_dek: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct EntryMetadata {
    pub id: Uuid,
    pub vault_id: Uuid,
    pub record_version: u64,
    pub created_at: i64,
    pub updated_at: i64,
    pub archived: bool,
}

#[derive(Debug)]
pub(crate) struct FieldEnvelope {
    pub field_id: FieldId,
    pub crypto_version: u16,
    pub record_version: u64,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub tag: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct StoredEntry {
    pub metadata: EntryMetadata,
    pub fields: Vec<FieldEnvelope>,
}

fn parse_uuid(value: &str) -> VaultResult<Uuid> {
    let uuid = Uuid::parse_str(value).map_err(|_| VaultError::UnsupportedFormat)?;
    if uuid.hyphenated().to_string() != value {
        return Err(VaultError::UnsupportedFormat);
    }
    Ok(uuid)
}

fn positive_u64(value: i64) -> VaultResult<u64> {
    u64::try_from(value)
        .ok()
        .filter(|value| *value >= 1)
        .ok_or(VaultError::UnsupportedFormat)
}

pub(crate) fn load_master_slot(conn: &Connection) -> VaultResult<Option<MasterSlot>> {
    let row = conn
        .query_row(
            "SELECT vault_id, crypto_version, kdf_profile, kdf_m_cost_kib,
                    kdf_t_cost, kdf_p_cost, kdf_salt, master_wrap_nonce, master_wrapped_dek
             FROM vault_meta WHERE id = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Vec<u8>>(6)?,
                    row.get::<_, Vec<u8>>(7)?,
                    row.get::<_, Vec<u8>>(8)?,
                ))
            },
        )
        .optional()?;
    let Some((
        vault_id,
        crypto_version,
        profile_name,
        m_cost,
        t_cost,
        p_cost,
        salt,
        wrap_nonce,
        wrapped_dek,
    )) = row
    else {
        return Ok(None);
    };
    if crypto_version != i64::from(CRYPTO_VERSION)
        || salt.len() != SALT_LEN
        || wrap_nonce.len() != NONCE_LEN
        || wrapped_dek.len() != 48
    {
        return Err(VaultError::UnsupportedFormat);
    }
    let profile = KdfProfile::from_metadata(
        &profile_name,
        u32::try_from(m_cost).map_err(|_| VaultError::UnsupportedFormat)?,
        u32::try_from(t_cost).map_err(|_| VaultError::UnsupportedFormat)?,
        u32::try_from(p_cost).map_err(|_| VaultError::UnsupportedFormat)?,
    )?;
    Ok(Some(MasterSlot {
        vault_id: parse_uuid(&vault_id)?,
        profile,
        salt,
        wrap_nonce,
        wrapped_dek,
    }))
}

pub(crate) fn insert_master_slot(
    tx: &Transaction<'_>,
    slot: &MasterSlot,
    now: i64,
) -> VaultResult<()> {
    let changed = tx.execute(
        "INSERT INTO vault_meta (
            id, vault_id, crypto_version, kdf_profile, kdf_m_cost_kib,
            kdf_t_cost, kdf_p_cost, kdf_salt, master_wrap_nonce,
            master_wrapped_dek, created_at, updated_at
         ) VALUES (1, ?1, 1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
        params![
            slot.vault_id.to_string(),
            KDF_PROFILE,
            slot.profile.m_cost_kib,
            slot.profile.t_cost,
            slot.profile.p_cost,
            slot.salt,
            slot.wrap_nonce,
            slot.wrapped_dek,
            now,
        ],
    )?;
    if changed != 1 {
        return Err(VaultError::AlreadyInitialized);
    }
    Ok(())
}

pub(crate) fn replace_master_slot(
    tx: &Transaction<'_>,
    slot: &MasterSlot,
    now: i64,
) -> VaultResult<()> {
    let changed = tx.execute(
        "UPDATE vault_meta SET kdf_profile = ?2, kdf_m_cost_kib = ?3,
            kdf_t_cost = ?4, kdf_p_cost = ?5, kdf_salt = ?6,
            master_wrap_nonce = ?7, master_wrapped_dek = ?8, updated_at = ?9
         WHERE id = 1 AND vault_id = ?1 AND crypto_version = 1",
        params![
            slot.vault_id.to_string(),
            KDF_PROFILE,
            slot.profile.m_cost_kib,
            slot.profile.t_cost,
            slot.profile.p_cost,
            slot.salt,
            slot.wrap_nonce,
            slot.wrapped_dek,
            now,
        ],
    )?;
    if changed != 1 {
        return Err(VaultError::DataIntegrityError);
    }
    Ok(())
}

pub(crate) fn load_used_nonces(conn: &Connection) -> VaultResult<HashSet<[u8; NONCE_LEN]>> {
    let mut stmt = conn.prepare("SELECT nonce FROM password_vault_fields")?;
    let rows = stmt.query_map([], |row| row.get::<_, Vec<u8>>(0))?;
    let mut nonces = HashSet::new();
    for row in rows {
        let nonce = row?;
        if nonce.len() != NONCE_LEN {
            return Err(VaultError::DataIntegrityError);
        }
        let mut value = [0u8; NONCE_LEN];
        value.copy_from_slice(&nonce);
        nonces.insert(value);
    }
    Ok(nonces)
}

fn read_metadata(row: &rusqlite::Row<'_>) -> rusqlite::Result<EntryMetadataRow> {
    Ok(EntryMetadataRow {
        id: row.get(0)?,
        vault_id: row.get(1)?,
        record_version: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
        archived: row.get(5)?,
    })
}

struct EntryMetadataRow {
    id: String,
    vault_id: String,
    record_version: i64,
    created_at: i64,
    updated_at: i64,
    archived: i64,
}

impl EntryMetadataRow {
    fn parse(self) -> VaultResult<EntryMetadata> {
        if !matches!(self.archived, 0 | 1) {
            return Err(VaultError::UnsupportedFormat);
        }
        Ok(EntryMetadata {
            id: parse_uuid(&self.id)?,
            vault_id: parse_uuid(&self.vault_id)?,
            record_version: positive_u64(self.record_version)?,
            created_at: self.created_at,
            updated_at: self.updated_at,
            archived: self.archived == 1,
        })
    }
}

const METADATA_SELECT: &str =
    "SELECT id, vault_id, record_version, created_at, updated_at, archived";

pub(crate) fn load_entry(conn: &Connection, entry_id: &Uuid) -> VaultResult<StoredEntry> {
    let metadata = conn
        .query_row(
            &format!("{METADATA_SELECT} FROM password_vault_entries WHERE id = ?1"),
            params![entry_id.to_string()],
            read_metadata,
        )
        .optional()?
        .ok_or(VaultError::NotFound)?
        .parse()?;
    if metadata.id != *entry_id {
        return Err(VaultError::DataIntegrityError);
    }
    let mut stmt = conn.prepare(
        "SELECT field_id, crypto_version, record_version, nonce, ciphertext, tag
         FROM password_vault_fields WHERE entry_id = ?1 ORDER BY field_id",
    )?;
    let rows = stmt.query_map(params![entry_id.to_string()], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, Vec<u8>>(3)?,
            row.get::<_, Vec<u8>>(4)?,
            row.get::<_, Vec<u8>>(5)?,
        ))
    })?;
    let mut fields = Vec::new();
    for row in rows {
        let (field_id, crypto_version, record_version, nonce, ciphertext, tag) = row?;
        let field_id =
            FieldId::from_u8(u8::try_from(field_id).map_err(|_| VaultError::DataIntegrityError)?)
                .ok_or(VaultError::DataIntegrityError)?;
        let crypto_version =
            u16::try_from(crypto_version).map_err(|_| VaultError::DataIntegrityError)?;
        if crypto_version != CRYPTO_VERSION
            || nonce.len() != NONCE_LEN
            || tag.len() != TAG_LEN
            || ciphertext.len() > crate::model::MAX_FIELD_BYTES
        {
            return Err(VaultError::DataIntegrityError);
        }
        fields.push(FieldEnvelope {
            field_id,
            crypto_version,
            record_version: positive_u64(record_version)?,
            nonce,
            ciphertext,
            tag,
        });
    }
    // v1 总是保存七个字段（包括空字符串）；缺字段不能按空值静默修复。
    if fields.len() != FieldId::ALL.len() {
        return Err(VaultError::DataIntegrityError);
    }
    Ok(StoredEntry { metadata, fields })
}

pub(crate) fn list_metadata(conn: &Connection) -> VaultResult<Vec<EntryMetadata>> {
    let mut stmt = conn.prepare(&format!(
        "{METADATA_SELECT} FROM password_vault_entries ORDER BY updated_at DESC, id DESC"
    ))?;
    let rows = stmt.query_map([], read_metadata)?;
    let mut records = Vec::new();
    for row in rows {
        records.push(row?.parse()?);
    }
    Ok(records)
}

pub(crate) fn insert_entry(
    tx: &Transaction<'_>,
    metadata: &EntryMetadata,
    fields: &[FieldEnvelope],
) -> VaultResult<()> {
    tx.execute(
        "INSERT INTO password_vault_entries (
            id, vault_id, record_version, created_at, updated_at, archived
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            metadata.id.to_string(),
            metadata.vault_id.to_string(),
            metadata.record_version,
            metadata.created_at,
            metadata.updated_at,
            metadata.archived,
        ],
    )?;
    insert_fields(tx, &metadata.id, fields)
}

fn insert_fields(
    tx: &Transaction<'_>,
    entry_id: &Uuid,
    fields: &[FieldEnvelope],
) -> VaultResult<()> {
    for field in fields {
        tx.execute(
            "INSERT INTO password_vault_fields (
                entry_id, field_id, crypto_version, record_version, nonce, ciphertext, tag
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                entry_id.to_string(),
                field.field_id as u8,
                field.crypto_version,
                field.record_version,
                field.nonce,
                field.ciphertext,
                field.tag,
            ],
        )?;
    }
    Ok(())
}

pub(crate) fn replace_entry(
    tx: &Transaction<'_>,
    expected_version: u64,
    metadata: &EntryMetadata,
    fields: &[FieldEnvelope],
) -> VaultResult<()> {
    let changed = tx.execute(
        "UPDATE password_vault_entries
         SET record_version = ?2, updated_at = ?3, archived = ?4
         WHERE id = ?1 AND record_version = ?5",
        params![
            metadata.id.to_string(),
            metadata.record_version,
            metadata.updated_at,
            metadata.archived,
            expected_version,
        ],
    )?;
    if changed != 1 {
        return Err(VaultError::VersionConflict);
    }
    tx.execute(
        "DELETE FROM password_vault_fields WHERE entry_id = ?1",
        params![metadata.id.to_string()],
    )?;
    insert_fields(tx, &metadata.id, fields)
}

pub(crate) fn delete_entry(
    tx: &Transaction<'_>,
    entry_id: &Uuid,
    expected_version: u64,
) -> VaultResult<()> {
    let changed = tx.execute(
        "DELETE FROM password_vault_entries WHERE id = ?1 AND record_version = ?2",
        params![entry_id.to_string(), expected_version],
    )?;
    if changed == 0 {
        let exists = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM password_vault_entries WHERE id = ?1)",
            params![entry_id.to_string()],
            |row| row.get::<_, bool>(0),
        )?;
        return Err(if exists {
            VaultError::VersionConflict
        } else {
            VaultError::NotFound
        });
    }
    Ok(())
}

/// 启动校验只读；任何 checksum、未来版本、结构或外键错误都 fail closed。
pub(crate) fn validate_storage(conn: &Connection) -> VaultResult<()> {
    let expected = migrations().remove(0).checksum_hex();
    let mut stmt = conn.prepare(
        "SELECT version, checksum FROM _schema_migrations WHERE module_id = ?1 ORDER BY version",
    )?;
    let rows = stmt.query_map(params![MODULE_ID], |row| {
        Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut seen_v1 = false;
    for row in rows {
        let (version, checksum) = row?;
        if version != 1 || checksum != expected || seen_v1 {
            return Err(VaultError::UnsupportedFormat);
        }
        seen_v1 = true;
    }
    if !seen_v1 {
        return Err(VaultError::UnsupportedFormat);
    }
    let integrity: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Err(VaultError::DataIntegrityError);
    }
    let mut fk = conn.prepare("PRAGMA foreign_key_check")?;
    if fk.query([])?.next()?.is_some() {
        return Err(VaultError::DataIntegrityError);
    }
    Ok(())
}
