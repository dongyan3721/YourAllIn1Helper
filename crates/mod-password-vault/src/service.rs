use crate::{
    crypto::{
        decrypt_field, derive_kek, encrypt_field, plaintext_to_string, random_nonce, random_uuid,
        unwrap_dek, wrap_dek, KdfProfile, OsRandom, VaultRandom, KEY_LEN, NONCE_LEN,
    },
    error::{VaultError, VaultResult},
    model::{
        FieldId, LockReason, PasswordPolicy, SecretString, VaultEntry, VaultEntryInput,
        VaultEntryPage, VaultEntrySummary, VaultEvent, VaultMutationResult, VaultStatus,
        AUTO_LOCK_SECONDS,
    },
    store::{self, EntryMetadata, FieldEnvelope, MasterSlot, StoredEntry},
};
use app_core::ModuleContext;
use chrono::Utc;
use parking_lot::{Mutex, RwLock};
use std::{collections::HashSet, sync::Arc};
use uuid::Uuid;
use zeroize::Zeroizing;

pub trait VaultEventSink: Send + Sync + 'static {
    fn locked(&self, event: VaultEvent);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VaultPhase {
    Uninitialized,
    Locked,
    Unlocking,
    Unlocked,
}

struct VaultState {
    phase: VaultPhase,
    epoch: u64,
    accepting: bool,
    vault_id: Option<Uuid>,
    dek: Option<Zeroizing<[u8; KEY_LEN]>>,
    used_nonces: HashSet<[u8; NONCE_LEN]>,
    capability_error: Option<String>,
}

impl VaultState {
    fn new() -> Self {
        Self {
            phase: VaultPhase::Uninitialized,
            epoch: 0,
            accepting: false,
            vault_id: None,
            dek: None,
            used_nonces: HashSet::new(),
            capability_error: None,
        }
    }

    fn status(&self) -> VaultStatus {
        VaultStatus {
            initialized: self.phase != VaultPhase::Uninitialized,
            locked: self.phase != VaultPhase::Unlocked,
            unlocking: self.phase == VaultPhase::Unlocking,
            epoch: self.epoch,
            auto_lock_seconds: AUTO_LOCK_SECONDS,
            biometric_available: false,
            secret_copy_available: false,
            capability_error: self.capability_error.clone(),
        }
    }
}

/// 主密码服务；密码学重任务通过 spawn_blocking 离开 UI/AppKit 线程。
pub struct VaultService {
    state: Mutex<VaultState>,
    random: Arc<dyn VaultRandom>,
    sink: RwLock<Option<Arc<dyn VaultEventSink>>>,
}

impl Default for VaultService {
    fn default() -> Self {
        Self::new()
    }
}

impl VaultService {
    pub fn new() -> Self {
        Self::with_random(Arc::new(OsRandom))
    }

    /// 显式测试/装配入口；生产 new 固定使用 OsRandom，不存在 Mock 回退。
    #[doc(hidden)]
    pub fn with_random(random: Arc<dyn VaultRandom>) -> Self {
        Self {
            state: Mutex::new(VaultState::new()),
            random,
            sink: RwLock::new(None),
        }
    }

    pub fn set_event_sink(&self, sink: Option<Arc<dyn VaultEventSink>>) {
        *self.sink.write() = sink;
    }

    fn emit(&self, event: Option<VaultEvent>) {
        if let (Some(event), Some(sink)) = (event, self.sink.read().as_ref()) {
            sink.locked(event);
        }
    }

    fn lock_state(state: &mut VaultState, reason: LockReason) -> Option<VaultEvent> {
        let was_active = matches!(state.phase, VaultPhase::Unlocking | VaultPhase::Unlocked);
        state.epoch = state.epoch.wrapping_add(1);
        state.dek.take();
        state.used_nonces.clear();
        state.phase = if state.vault_id.is_some() {
            VaultPhase::Locked
        } else {
            VaultPhase::Uninitialized
        };
        was_active.then_some(VaultEvent {
            epoch: state.epoch,
            reason,
        })
    }

    pub fn lock(&self, reason: LockReason) -> VaultStatus {
        let event = {
            let mut state = self.state.lock();
            Self::lock_state(&mut state, reason)
        };
        self.emit(event);
        self.status()
    }

    pub fn status(&self) -> VaultStatus {
        self.state.lock().status()
    }

    pub(crate) fn set_accepting(&self, accepting: bool) {
        self.state.lock().accepting = accepting;
    }

    pub(crate) fn record_capability_result(&self, error: Option<String>) {
        self.state.lock().capability_error = error;
    }

    pub(crate) fn refresh_from_disk(&self, ctx: &ModuleContext) -> VaultResult<()> {
        let conn = ctx.db.lock();
        store::validate_storage(&conn)?;
        let slot = store::load_master_slot(&conn)?;
        let used_nonces = store::load_used_nonces(&conn)?;
        drop(conn);
        let mut state = self.state.lock();
        state.dek.take();
        state.used_nonces = used_nonces;
        state.vault_id = slot.map(|slot| slot.vault_id);
        state.phase = if state.vault_id.is_some() {
            VaultPhase::Locked
        } else {
            VaultPhase::Uninitialized
        };
        state.epoch = state.epoch.wrapping_add(1);
        Ok(())
    }

    fn ensure_accepting(state: &VaultState) -> VaultResult<()> {
        if state.accepting {
            Ok(())
        } else {
            Err(VaultError::Disabled)
        }
    }

    fn validate_new_password(password: &str) -> VaultResult<()> {
        let chars = password.chars().count();
        if !(12..=256).contains(&chars)
            || password.len() > 1024
            || password.chars().all(char::is_whitespace)
        {
            return Err(VaultError::InvalidInput(
                "master password must contain 12 to 256 characters and not be whitespace-only",
            ));
        }
        Ok(())
    }

    pub async fn initialize(
        self: &Arc<Self>,
        ctx: ModuleContext,
        password: Zeroizing<String>,
        confirmation: Zeroizing<String>,
    ) -> VaultResult<VaultStatus> {
        Self::validate_new_password(&password)?;
        if password != confirmation {
            return Err(VaultError::InvalidInput(
                "master password confirmation differs",
            ));
        }
        let service = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            let mut state = service.state.lock();
            Self::ensure_accepting(&state)?;
            if state.phase != VaultPhase::Uninitialized {
                return Err(VaultError::AlreadyInitialized);
            }
            {
                let conn = ctx.db.lock();
                if store::load_master_slot(&conn)?.is_some() {
                    return Err(VaultError::AlreadyInitialized);
                }
            }
            let vault_id = random_uuid(service.random.as_ref())?;
            let dek = Zeroizing::new({
                let mut bytes = [0u8; KEY_LEN];
                service.random.fill(&mut bytes)?;
                bytes
            });
            let mut salt = [0u8; 16];
            service.random.fill(&mut salt)?;
            let wrap_nonce = random_nonce(service.random.as_ref())?;
            let kek = derive_kek(KdfProfile::V1, password.as_bytes(), &salt)?;
            let wrapped = wrap_dek(&kek, &vault_id, &dek, &wrap_nonce)?;
            let slot = MasterSlot {
                vault_id,
                profile: KdfProfile::V1,
                salt: salt.to_vec(),
                wrap_nonce: wrap_nonce.to_vec(),
                wrapped_dek: wrapped.to_vec(),
            };
            let mut conn = ctx.db.lock();
            let tx = conn.transaction()?;
            store::insert_master_slot(&tx, &slot, Utc::now().timestamp_millis())?;
            tx.commit()?;
            state.vault_id = Some(vault_id);
            state.phase = VaultPhase::Locked;
            state.epoch = state.epoch.wrapping_add(1);
            Ok(state.status())
        })
        .await
        .map_err(|_| VaultError::TaskJoin)?
    }

    pub async fn unlock_master_password(
        self: &Arc<Self>,
        ctx: ModuleContext,
        password: Zeroizing<String>,
    ) -> VaultResult<VaultStatus> {
        let (attempt, slot) = {
            let mut state = self.state.lock();
            Self::ensure_accepting(&state)?;
            match state.phase {
                VaultPhase::Uninitialized => return Err(VaultError::Uninitialized),
                VaultPhase::Unlocking => return Err(VaultError::UnlockInProgress),
                VaultPhase::Unlocked => return Err(VaultError::InvalidInput("already unlocked")),
                VaultPhase::Locked => {}
            }
            let conn = ctx.db.lock();
            let slot = store::load_master_slot(&conn)?.ok_or(VaultError::Uninitialized)?;
            state.phase = VaultPhase::Unlocking;
            state.epoch = state.epoch.wrapping_add(1);
            (state.epoch, slot)
        };
        let task = tokio::task::spawn_blocking(move || {
            let kek = derive_kek(slot.profile, password.as_bytes(), &slot.salt)?;
            unwrap_dek(&kek, &slot.vault_id, &slot.wrap_nonce, &slot.wrapped_dek)
        });
        let result = match task.await {
            Ok(result) => result,
            Err(_) => {
                let mut state = self.state.lock();
                if state.phase == VaultPhase::Unlocking && state.epoch == attempt {
                    state.phase = VaultPhase::Locked;
                }
                return Err(VaultError::TaskJoin);
            }
        };
        let event;
        let mut state = self.state.lock();
        if state.phase != VaultPhase::Unlocking || state.epoch != attempt || !state.accepting {
            // 迟到 KEK/DEK 随 result 在本作用域 Drop 清零，绝不恢复旧会话。
            return Err(VaultError::Locked);
        }
        match result {
            Ok(dek) => {
                let used_nonces = {
                    let conn = ctx.db.lock();
                    match store::load_used_nonces(&conn) {
                        Ok(nonces) => nonces,
                        Err(_) => {
                            event = Self::lock_state(&mut state, LockReason::IntegrityError);
                            drop(state);
                            self.emit(event);
                            return Err(VaultError::DataIntegrityError);
                        }
                    }
                };
                state.dek = Some(dek);
                state.used_nonces = used_nonces;
                state.phase = VaultPhase::Unlocked;
                state.capability_error = None;
                Ok(state.status())
            }
            Err(_) => {
                state.phase = VaultPhase::Locked;
                state.dek.take();
                state.used_nonces.clear();
                Err(VaultError::UnlockFailed)
            }
        }
    }

    fn unlocked_parts(state: &VaultState) -> VaultResult<(Uuid, &[u8; KEY_LEN], u64)> {
        Self::ensure_accepting(state)?;
        if state.phase != VaultPhase::Unlocked {
            return Err(VaultError::Locked);
        }
        Ok((
            state.vault_id.ok_or(VaultError::DataIntegrityError)?,
            state.dek.as_deref().ok_or(VaultError::Locked)?,
            state.epoch,
        ))
    }

    fn check_context(metadata: &EntryMetadata, vault_id: &Uuid) -> VaultResult<()> {
        if metadata.vault_id != *vault_id {
            Err(VaultError::DataIntegrityError)
        } else {
            Ok(())
        }
    }

    fn validate_envelope_versions(stored: &StoredEntry) -> VaultResult<()> {
        if stored.fields.iter().any(|field| {
            field.record_version != stored.metadata.record_version
                || field.crypto_version != crate::crypto::CRYPTO_VERSION
        }) {
            return Err(VaultError::DataIntegrityError);
        }
        Ok(())
    }

    fn decrypt_one(
        dek: &[u8; KEY_LEN],
        stored: &StoredEntry,
        wanted: FieldId,
    ) -> VaultResult<Zeroizing<Vec<u8>>> {
        let field = stored
            .fields
            .iter()
            .find(|field| field.field_id as u8 == wanted as u8)
            .ok_or(VaultError::DataIntegrityError)?;
        decrypt_field(
            dek,
            &stored.metadata.vault_id,
            &stored.metadata.id,
            wanted,
            field.crypto_version,
            field.record_version,
            &field.nonce,
            &field.ciphertext,
            &field.tag,
        )
    }

    fn decrypt_entry(dek: &[u8; KEY_LEN], stored: &StoredEntry) -> VaultResult<VaultEntry> {
        Self::validate_envelope_versions(stored)?;
        let mut values: [Option<SecretString>; 7] = Default::default();
        for field in FieldId::ALL {
            let plaintext = Self::decrypt_one(dek, stored, field)?;
            values[field as usize - 1] = Some(plaintext_to_string(plaintext)?);
        }
        let mut take = |field: FieldId| {
            values[field as usize - 1]
                .take()
                .ok_or(VaultError::DataIntegrityError)
        };
        Ok(VaultEntry {
            id: stored.metadata.id.to_string(),
            record_version: stored.metadata.record_version,
            created_at: stored.metadata.created_at,
            updated_at: stored.metadata.updated_at,
            archived: stored.metadata.archived,
            title: take(FieldId::Title)?,
            url: take(FieldId::Url)?,
            category: take(FieldId::Category)?,
            username: take(FieldId::Username)?,
            password: take(FieldId::Password)?,
            totp_secret: take(FieldId::TotpSecret)?,
            notes: take(FieldId::Notes)?,
        })
    }

    fn decrypt_summary(
        dek: &[u8; KEY_LEN],
        stored: &StoredEntry,
    ) -> VaultResult<(VaultEntrySummary, SecretString)> {
        Self::validate_envelope_versions(stored)?;
        let text = |field| -> VaultResult<SecretString> {
            plaintext_to_string(Self::decrypt_one(dek, stored, field)?)
        };
        Ok((
            VaultEntrySummary {
                id: stored.metadata.id.to_string(),
                record_version: stored.metadata.record_version,
                created_at: stored.metadata.created_at,
                updated_at: stored.metadata.updated_at,
                archived: stored.metadata.archived,
                title: text(FieldId::Title)?,
                url: text(FieldId::Url)?,
                category: text(FieldId::Category)?,
                username: text(FieldId::Username)?,
            },
            text(FieldId::Notes)?,
        ))
    }

    fn integrity_event(
        &self,
        state: &mut VaultState,
        result: &VaultResult<impl Sized>,
    ) -> Option<VaultEvent> {
        if matches!(
            result,
            Err(VaultError::DataIntegrityError | VaultError::UnsupportedFormat)
        ) {
            Self::lock_state(state, LockReason::IntegrityError)
        } else {
            None
        }
    }

    fn encrypt_fields(
        &self,
        state: &mut VaultState,
        vault_id: &Uuid,
        entry_id: &Uuid,
        record_version: u64,
        input: &VaultEntryInput,
    ) -> VaultResult<Vec<FieldEnvelope>> {
        let dek = state.dek.as_deref().ok_or(VaultError::Locked)?;
        let mut fields = Vec::with_capacity(FieldId::ALL.len());
        for field_id in FieldId::ALL {
            let nonce = random_nonce(self.random.as_ref())?;
            if !state.used_nonces.insert(nonce) {
                return Err(VaultError::NonceReuse);
            }
            let plaintext = Zeroizing::new(input.field(field_id).as_bytes().to_vec());
            let (ciphertext, tag) = encrypt_field(
                dek,
                vault_id,
                entry_id,
                field_id,
                record_version,
                &plaintext,
                &nonce,
            )?;
            fields.push(FieldEnvelope {
                field_id,
                crypto_version: crate::crypto::CRYPTO_VERSION,
                record_version,
                nonce: nonce.to_vec(),
                ciphertext,
                tag: tag.to_vec(),
            });
        }
        Ok(fields)
    }

    pub async fn create_entry(
        self: &Arc<Self>,
        ctx: ModuleContext,
        input: VaultEntryInput,
    ) -> VaultResult<VaultMutationResult> {
        input.validate().map_err(VaultError::InvalidInput)?;
        let service = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            let mut state = service.state.lock();
            let (vault_id, _, _) = Self::unlocked_parts(&state)?;
            let result = (|| {
                let entry_id = random_uuid(service.random.as_ref())?;
                let now = Utc::now().timestamp_millis();
                let metadata = EntryMetadata {
                    id: entry_id,
                    vault_id,
                    record_version: 1,
                    created_at: now,
                    updated_at: now,
                    archived: input.archived,
                };
                let fields = service.encrypt_fields(&mut state, &vault_id, &entry_id, 1, &input)?;
                let mut conn = ctx.db.lock();
                let tx = conn.transaction()?;
                store::insert_entry(&tx, &metadata, &fields)?;
                tx.commit()?;
                Ok(VaultMutationResult {
                    id: entry_id.to_string(),
                    record_version: 1,
                })
            })();
            let event = service.integrity_event(&mut state, &result);
            drop(state);
            service.emit(event);
            result
        })
        .await
        .map_err(|_| VaultError::TaskJoin)?
    }

    pub async fn update_entry(
        self: &Arc<Self>,
        ctx: ModuleContext,
        entry_id: &str,
        expected_version: u64,
        input: VaultEntryInput,
    ) -> VaultResult<VaultMutationResult> {
        input.validate().map_err(VaultError::InvalidInput)?;
        let entry_id = parse_entry_id(entry_id)?;
        let service = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            let mut state = service.state.lock();
            let (vault_id, _, _) = Self::unlocked_parts(&state)?;
            let result = (|| {
                let mut conn = ctx.db.lock();
                let tx = conn.transaction()?;
                let stored = store::load_entry(&tx, &entry_id)?;
                Self::check_context(&stored.metadata, &vault_id)?;
                if stored.metadata.record_version != expected_version {
                    return Err(VaultError::VersionConflict);
                }
                let dek = state.dek.as_deref().ok_or(VaultError::Locked)?;
                // 即使完整 upsert 替换所有值，也先认证旧记录，禁止静默绕过损坏。
                for field in FieldId::ALL {
                    let plaintext = Self::decrypt_one(dek, &stored, field)?;
                    drop(plaintext);
                }
                let record_version = expected_version
                    .checked_add(1)
                    .ok_or(VaultError::InvalidInput("record version overflow"))?;
                let metadata = EntryMetadata {
                    record_version,
                    updated_at: Utc::now().timestamp_millis(),
                    archived: input.archived,
                    ..stored.metadata
                };
                let fields = service.encrypt_fields(
                    &mut state,
                    &vault_id,
                    &entry_id,
                    record_version,
                    &input,
                )?;
                store::replace_entry(&tx, expected_version, &metadata, &fields)?;
                tx.commit()?;
                Ok(VaultMutationResult {
                    id: entry_id.to_string(),
                    record_version,
                })
            })();
            let event = service.integrity_event(&mut state, &result);
            drop(state);
            service.emit(event);
            result
        })
        .await
        .map_err(|_| VaultError::TaskJoin)?
    }

    pub async fn delete_entry(
        self: &Arc<Self>,
        ctx: ModuleContext,
        entry_id: &str,
        expected_version: u64,
    ) -> VaultResult<()> {
        let entry_id = parse_entry_id(entry_id)?;
        let service = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            let state = service.state.lock();
            Self::unlocked_parts(&state)?;
            let mut conn = ctx.db.lock();
            let tx = conn.transaction()?;
            store::delete_entry(&tx, &entry_id, expected_version)?;
            tx.commit()?;
            Ok(())
        })
        .await
        .map_err(|_| VaultError::TaskJoin)?
    }

    pub async fn get_entry(
        self: &Arc<Self>,
        ctx: ModuleContext,
        entry_id: &str,
    ) -> VaultResult<VaultEntry> {
        let entry_id = parse_entry_id(entry_id)?;
        let service = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            let mut state = service.state.lock();
            let result = (|| {
                let (vault_id, dek, _) = Self::unlocked_parts(&state)?;
                let conn = ctx.db.lock();
                let stored = store::load_entry(&conn, &entry_id)?;
                Self::check_context(&stored.metadata, &vault_id)?;
                Self::decrypt_entry(dek, &stored)
            })();
            let event = service.integrity_event(&mut state, &result);
            drop(state);
            service.emit(event);
            result
        })
        .await
        .map_err(|_| VaultError::TaskJoin)?
    }

    pub async fn reveal_password(
        self: &Arc<Self>,
        ctx: ModuleContext,
        entry_id: &str,
    ) -> VaultResult<SecretString> {
        let entry_id = parse_entry_id(entry_id)?;
        let service = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            let mut state = service.state.lock();
            let result = (|| {
                let (vault_id, dek, _) = Self::unlocked_parts(&state)?;
                let conn = ctx.db.lock();
                let stored = store::load_entry(&conn, &entry_id)?;
                Self::check_context(&stored.metadata, &vault_id)?;
                Self::validate_envelope_versions(&stored)?;
                plaintext_to_string(Self::decrypt_one(dek, &stored, FieldId::Password)?)
            })();
            let event = service.integrity_event(&mut state, &result);
            drop(state);
            service.emit(event);
            result
        })
        .await
        .map_err(|_| VaultError::TaskJoin)?
    }

    pub async fn list_entries(
        self: &Arc<Self>,
        ctx: ModuleContext,
        search: Option<String>,
        cursor: Option<String>,
        limit: Option<u32>,
    ) -> VaultResult<VaultEntryPage> {
        let limit = usize::try_from(limit.unwrap_or(50).clamp(1, 100))
            .map_err(|_| VaultError::InvalidInput("invalid page limit"))?;
        let offset = match cursor {
            Some(cursor) => cursor
                .parse::<usize>()
                .map_err(|_| VaultError::InvalidInput("invalid cursor"))?,
            None => 0,
        };
        let needle = search.unwrap_or_default().trim().to_lowercase();
        if needle.len() > 1024 {
            return Err(VaultError::InvalidInput("search is too long"));
        }
        let service = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            let mut state = service.state.lock();
            let result = (|| {
                let (vault_id, dek, _) = Self::unlocked_parts(&state)?;
                let conn = ctx.db.lock();
                let metadata = store::list_metadata(&conn)?;
                let mut matched = Vec::new();
                let mut next_cursor = None;
                for item in metadata {
                    Self::check_context(&item, &vault_id)?;
                    let stored = store::load_entry(&conn, &item.id)?;
                    let (summary, notes) = Self::decrypt_summary(dek, &stored)?;
                    let matches = needle.is_empty()
                        || summary.title.to_lowercase().contains(&needle)
                        || summary.username.to_lowercase().contains(&needle)
                        || summary.url.to_lowercase().contains(&needle)
                        || summary.category.to_lowercase().contains(&needle)
                        || notes.to_lowercase().contains(&needle);
                    if !matches {
                        continue;
                    }
                    if matched.len() < offset {
                        matched.push(summary);
                        continue;
                    }
                    if matched.len() == offset + limit {
                        next_cursor = Some((offset + limit).to_string());
                        break;
                    }
                    matched.push(summary);
                }
                let entries = matched.into_iter().skip(offset).collect();
                Ok(VaultEntryPage {
                    entries,
                    next_cursor,
                })
            })();
            let event = service.integrity_event(&mut state, &result);
            drop(state);
            service.emit(event);
            result
        })
        .await
        .map_err(|_| VaultError::TaskJoin)?
    }

    pub async fn change_master_password(
        self: &Arc<Self>,
        ctx: ModuleContext,
        old_password: Zeroizing<String>,
        new_password: Zeroizing<String>,
    ) -> VaultResult<VaultStatus> {
        Self::validate_new_password(&new_password)?;
        let service = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            // 整个重包操作线性化；lock 等待本操作结束后才能返回，不留迟到 DEK。
            let mut state = service.state.lock();
            let result = (|| {
                let (vault_id, dek, _) = Self::unlocked_parts(&state)?;
                let slot = {
                    let conn = ctx.db.lock();
                    store::load_master_slot(&conn)?.ok_or(VaultError::Uninitialized)?
                };
                if slot.vault_id != vault_id {
                    return Err(VaultError::DataIntegrityError);
                }
                let old_kek = derive_kek(slot.profile, old_password.as_bytes(), &slot.salt)?;
                let verified = unwrap_dek(
                    &old_kek,
                    &slot.vault_id,
                    &slot.wrap_nonce,
                    &slot.wrapped_dek,
                )?;
                if verified.as_slice() != dek.as_slice() {
                    return Err(VaultError::UnlockFailed);
                }
                let mut salt = [0u8; 16];
                service.random.fill(&mut salt)?;
                let wrap_nonce = random_nonce(service.random.as_ref())?;
                let new_kek = derive_kek(KdfProfile::V1, new_password.as_bytes(), &salt)?;
                let wrapped = wrap_dek(&new_kek, &vault_id, dek, &wrap_nonce)?;
                let replacement = MasterSlot {
                    vault_id,
                    profile: KdfProfile::V1,
                    salt: salt.to_vec(),
                    wrap_nonce: wrap_nonce.to_vec(),
                    wrapped_dek: wrapped.to_vec(),
                };
                let mut conn = ctx.db.lock();
                let tx = conn.transaction()?;
                store::replace_master_slot(&tx, &replacement, Utc::now().timestamp_millis())?;
                tx.commit()?;
                Ok(())
            })();
            if result.is_err() {
                let event = service.integrity_event(&mut state, &result);
                drop(state);
                service.emit(event);
                return Err(result.err().unwrap_or(VaultError::DataIntegrityError));
            }
            let event = Self::lock_state(&mut state, LockReason::PasswordChanged);
            let status = state.status();
            drop(state);
            service.emit(event);
            Ok(status)
        })
        .await
        .map_err(|_| VaultError::TaskJoin)?
    }

    fn random_index(&self, upper: usize) -> VaultResult<usize> {
        if upper == 0 || upper > 256 {
            return Err(VaultError::InvalidInput("invalid random range"));
        }
        let acceptance = (256 / upper) * upper;
        loop {
            let mut byte = [0u8; 1];
            self.random.fill(&mut byte)?;
            if usize::from(byte[0]) < acceptance {
                return Ok(usize::from(byte[0]) % upper);
            }
        }
    }

    pub fn generate_password(&self, policy: PasswordPolicy) -> VaultResult<Zeroizing<String>> {
        let length = usize::from(policy.length);
        if !(8..=256).contains(&length) {
            return Err(VaultError::InvalidInput("password length must be 8 to 256"));
        }
        let mut groups = Vec::new();
        if policy.uppercase {
            groups.push(b"ABCDEFGHJKLMNPQRSTUVWXYZ".as_slice());
        }
        if policy.lowercase {
            groups.push(b"abcdefghijkmnopqrstuvwxyz".as_slice());
        }
        if policy.digits {
            groups.push(b"23456789".as_slice());
        }
        if policy.symbols {
            groups.push(b"!@#$%^&*()-_=+[]{}?".as_slice());
        }
        if groups.is_empty() || length < groups.len() {
            return Err(VaultError::InvalidInput(
                "invalid password character policy",
            ));
        }
        // 参数错误可无秘密地直接返回；实际生成秘密前必须验证当前会话已解锁。
        Self::unlocked_parts(&self.state.lock())?;
        let alphabet: Vec<u8> = groups.concat();
        let mut output = Zeroizing::new(Vec::with_capacity(length));
        for group in &groups {
            output.push(group[self.random_index(group.len())?]);
        }
        while output.len() < length {
            output.push(alphabet[self.random_index(alphabet.len())?]);
        }
        for index in (1..output.len()).rev() {
            let other = self.random_index(index + 1)?;
            output.swap(index, other);
        }
        let value = String::from_utf8(std::mem::take(&mut *output))
            .map_err(|_| VaultError::DataIntegrityError)?;
        Ok(Zeroizing::new(value))
    }

    /// M1 明确禁用；不提供文本比较、AtomicBool 或直接剪贴板降级路径。
    pub fn copy_password_disabled(&self) -> VaultResult<()> {
        Err(VaultError::FeatureDisabled(
            "shared clipboard ownership lease is not reviewed",
        ))
    }

    /// M1 不读取认证结果、不写 Keychain/DPAPI，也不暴露设备槽开关。
    pub fn biometric_disabled(&self) -> VaultResult<()> {
        Err(VaultError::FeatureDisabled(
            "biometric device key protection is an M2 capability",
        ))
    }
}

fn parse_entry_id(value: &str) -> VaultResult<Uuid> {
    let uuid = Uuid::parse_str(value).map_err(|_| VaultError::InvalidInput("invalid entry id"))?;
    if uuid.hyphenated().to_string() != value {
        return Err(VaultError::InvalidInput("non-canonical entry id"));
    }
    Ok(uuid)
}
