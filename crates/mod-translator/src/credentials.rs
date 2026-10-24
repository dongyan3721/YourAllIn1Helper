//! 翻译服务凭据管理契约与安全内存存储。
//!
//! 安全与隐私要求：
//! 1. API Key 严禁以明文形式写入配置文件（config.json）、SQLite 数据库或运行时日志。
//! 2. 外部原生凭据库（如 macOS Keychain / Windows Credential Locker）接口由系统架构统一审查；
//!    在专属凭据库打通前，本模块采用安全的内存会话存储（SessionCredentialStore），暂禁用明文落盘持久化。
//! 3. 不擅自读取用户现存系统密钥，不假装接口替身具备系统级安全持久化，缺失凭据时真实报告。

use app_core::CoreResult;
use parking_lot::RwLock;
use std::{collections::HashMap, sync::Arc};

/// 凭据存储抽象接口（依赖注入）。
pub trait CredentialStorage: Send + Sync + 'static {
    /// 查询指定服务提供商的 API Key（敏感信息，仅用于网络调用构建头，禁止广播或写入日志）。
    fn get_api_key(&self, provider_id: &str) -> CoreResult<Option<String>>;

    /// 设置指定服务提供商的 API Key。
    fn set_api_key(&self, provider_id: &str, key: &str) -> CoreResult<()>;

    /// 删除指定服务提供商的 API Key。
    fn delete_api_key(&self, provider_id: &str) -> CoreResult<()>;

    /// 检查指定服务提供商是否已配置可用凭据（不暴露实际密钥值）。
    fn has_api_key(&self, provider_id: &str) -> bool;
}

/// 内存会话级安全凭据存储实现。
///
/// 特性：
/// - 线程安全；
/// - 生命周期随主程序会话结束自动清空；
/// - 严禁序列化写入磁盘或包含在 Debug/Display 中输出。
#[derive(Clone, Default)]
pub struct SessionCredentialStore {
    keys: Arc<RwLock<HashMap<String, String>>>,
}

impl SessionCredentialStore {
    pub fn new() -> Self {
        Self {
            keys: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 清空所有内存中的凭据。
    pub fn clear(&self) {
        self.keys.write().clear();
    }
}

impl std::fmt::Debug for SessionCredentialStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self.keys.read().len();
        f.debug_struct("SessionCredentialStore")
            .field("stored_providers_count", &count)
            .finish()
    }
}

impl CredentialStorage for SessionCredentialStore {
    fn get_api_key(&self, provider_id: &str) -> CoreResult<Option<String>> {
        Ok(self.keys.read().get(provider_id).cloned())
    }

    fn set_api_key(&self, provider_id: &str, key: &str) -> CoreResult<()> {
        let trimmed = key.trim();
        if trimmed.is_empty() {
            self.keys.write().remove(provider_id);
        } else {
            self.keys
                .write()
                .insert(provider_id.to_string(), trimmed.to_string());
        }
        Ok(())
    }

    fn delete_api_key(&self, provider_id: &str) -> CoreResult<()> {
        self.keys.write().remove(provider_id);
        Ok(())
    }

    fn has_api_key(&self, provider_id: &str) -> bool {
        self.keys
            .read()
            .get(provider_id)
            .map(|k| !k.is_empty())
            .unwrap_or(false)
    }
}
