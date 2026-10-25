//! 翻译 API Key 的凭据存储接线。
//!
//! - 默认（无桌面测试、钥匙串未接入）：会话内存存储，退出即清空。
//! - 生产 Tauri 进程：切换为系统钥匙串（`os_hal::SecretStore`），密钥不写入配置、数据库或日志。
//! 切换只发生在启动阶段，切换后不迁移会话内存中的旧值（启动时内存为空）。
use app_core::{CoreError, CoreResult};
use mod_translator::credentials::{CredentialStorage, SessionCredentialStore};
use os_hal::{HalError, SecretStore};
use parking_lot::RwLock;
use serde::Serialize;
use std::{collections::HashMap, sync::Arc};

/// 钥匙串条目的服务名；账户为翻译服务提供商 ID。
pub(crate) const TRANSLATOR_KEYCHAIN_SERVICE: &str = "com.allinone.desktop.translator";

/// 当前凭据后端说明，供界面如实显示保存位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialBackend {
    Session,
    SystemKeychain,
}

/// 可在启动时切换后端的凭据存储，模块始终持有同一个实例。
pub struct HostCredentialStore {
    backend: RwLock<(CredentialBackend, Arc<dyn CredentialStorage>)>,
}

impl HostCredentialStore {
    pub fn new_session() -> Self {
        Self {
            backend: RwLock::new((
                CredentialBackend::Session,
                Arc::new(SessionCredentialStore::new()),
            )),
        }
    }

    /// 切换到系统凭据库；凭据库不可用时保持会话存储并返回 `false`。
    /// `service` 区分生产与隔离 GUI 测试，避免 QA 副本读写日常使用的条目。
    pub fn use_system_store(&self, store: Arc<dyn SecretStore>, service: &str) -> bool {
        if !store.is_available() {
            return false;
        }
        *self.backend.write() = (
            CredentialBackend::SystemKeychain,
            Arc::new(KeychainCredentialStore::new(store, service)),
        );
        true
    }

    pub fn backend_kind(&self) -> CredentialBackend {
        self.backend.read().0
    }

    fn current(&self) -> Arc<dyn CredentialStorage> {
        self.backend.read().1.clone()
    }
}

impl CredentialStorage for HostCredentialStore {
    fn get_api_key(&self, provider_id: &str) -> CoreResult<Option<String>> {
        self.current().get_api_key(provider_id)
    }

    fn set_api_key(&self, provider_id: &str, key: &str) -> CoreResult<()> {
        self.current().set_api_key(provider_id, key)
    }

    fn delete_api_key(&self, provider_id: &str) -> CoreResult<()> {
        self.current().delete_api_key(provider_id)
    }

    fn has_api_key(&self, provider_id: &str) -> bool {
        self.current().has_api_key(provider_id)
    }
}

/// 基于系统钥匙串的凭据存储；「是否已配置」结果缓存在内存中，避免列表刷新反复访问钥匙串。
struct KeychainCredentialStore {
    store: Arc<dyn SecretStore>,
    service: String,
    presence: RwLock<HashMap<String, bool>>,
}

impl KeychainCredentialStore {
    fn new(store: Arc<dyn SecretStore>, service: &str) -> Self {
        Self {
            store,
            service: service.to_string(),
            presence: RwLock::new(HashMap::new()),
        }
    }
}

fn hal_to_core(error: HalError) -> CoreError {
    CoreError::Internal(anyhow::anyhow!("{}", error))
}

impl CredentialStorage for KeychainCredentialStore {
    fn get_api_key(&self, provider_id: &str) -> CoreResult<Option<String>> {
        let value = self
            .store
            .get_secret(&self.service, provider_id)
            .map_err(hal_to_core)?;
        let key = match value {
            Some(bytes) => Some(String::from_utf8(bytes).map_err(|_| {
                CoreError::Internal(anyhow::anyhow!("钥匙串中的 API Key 不是有效 UTF-8"))
            })?),
            None => None,
        };
        self.presence
            .write()
            .insert(provider_id.to_string(), key.is_some());
        Ok(key)
    }

    fn set_api_key(&self, provider_id: &str, key: &str) -> CoreResult<()> {
        let trimmed = key.trim();
        if trimmed.is_empty() {
            return self.delete_api_key(provider_id);
        }
        self.store
            .set_secret(&self.service, provider_id, trimmed.as_bytes())
            .map_err(hal_to_core)?;
        self.presence.write().insert(provider_id.to_string(), true);
        Ok(())
    }

    fn delete_api_key(&self, provider_id: &str) -> CoreResult<()> {
        self.store
            .delete_secret(&self.service, provider_id)
            .map_err(hal_to_core)?;
        self.presence.write().insert(provider_id.to_string(), false);
        Ok(())
    }

    fn has_api_key(&self, provider_id: &str) -> bool {
        if let Some(known) = self.presence.read().get(provider_id) {
            return *known;
        }
        // 读取失败（如用户拒绝钥匙串访问）按「未配置」处理并记录，不把错误当作已配置。
        match self.get_api_key(provider_id) {
            Ok(value) => value.is_some(),
            Err(error) => {
                tracing::warn!(provider_id, %error, "Keychain lookup failed for translator key");
                false
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/secure_credentials.rs"]
mod tests;
