//! 剪贴板所有权与凭据库的显式测试适配器（仅用于自动化测试，生产不回退到此实现）。
use crate::{
    secure::{ClipboardOwnership, ClipboardWriteToken, SecretStore},
    HalError, HalResult,
};
use parking_lot::Mutex;
use std::collections::HashMap;

#[derive(Debug, Default)]
struct ClipboardState {
    sequence: i64,
    text: Option<String>,
    concealed: bool,
}

/// 内存版剪贴板与凭据库，可模拟「其他程序修改剪贴板」与凭据库不可用。
#[derive(Debug, Default)]
pub struct MockSecureServices {
    clipboard: Mutex<ClipboardState>,
    secrets: Mutex<HashMap<(String, String), Vec<u8>>>,
    secret_store_unavailable: bool,
}

impl MockSecureServices {
    pub fn new() -> Self {
        Self::default()
    }

    /// 构造一个不可用的凭据库，用于测试调用方的降级提示。
    pub fn without_secret_store() -> Self {
        Self {
            secret_store_unavailable: true,
            ..Self::default()
        }
    }

    /// 模拟用户或其他程序写入剪贴板。
    pub fn simulate_external_copy(&self, text: &str) {
        let mut state = self.clipboard.lock();
        state.sequence += 1;
        state.text = Some(text.to_string());
        state.concealed = false;
    }

    /// 当前剪贴板文本与是否带敏感标记。
    pub fn clipboard_snapshot(&self) -> (Option<String>, bool) {
        let state = self.clipboard.lock();
        (state.text.clone(), state.concealed)
    }
}

impl ClipboardOwnership for MockSecureServices {
    fn write_concealed_text(&self, text: &str) -> HalResult<ClipboardWriteToken> {
        let mut state = self.clipboard.lock();
        state.sequence += 1;
        state.text = Some(text.to_string());
        state.concealed = true;
        Ok(ClipboardWriteToken {
            sequence: state.sequence,
        })
    }

    fn clear_if_unchanged(&self, token: &ClipboardWriteToken) -> HalResult<bool> {
        let mut state = self.clipboard.lock();
        if state.sequence != token.sequence {
            return Ok(false);
        }
        state.sequence += 1;
        state.text = None;
        state.concealed = false;
        Ok(true)
    }
}

impl SecretStore for MockSecureServices {
    fn is_available(&self) -> bool {
        !self.secret_store_unavailable
    }

    fn get_secret(&self, service: &str, account: &str) -> HalResult<Option<Vec<u8>>> {
        if self.secret_store_unavailable {
            return Err(HalError::Unsupported("system_secret_store"));
        }
        Ok(self
            .secrets
            .lock()
            .get(&(service.to_string(), account.to_string()))
            .cloned())
    }

    fn set_secret(&self, service: &str, account: &str, secret: &[u8]) -> HalResult<()> {
        if self.secret_store_unavailable {
            return Err(HalError::Unsupported("system_secret_store"));
        }
        self.secrets
            .lock()
            .insert((service.to_string(), account.to_string()), secret.to_vec());
        Ok(())
    }

    fn delete_secret(&self, service: &str, account: &str) -> HalResult<()> {
        if self.secret_store_unavailable {
            return Err(HalError::Unsupported("system_secret_store"));
        }
        self.secrets
            .lock()
            .remove(&(service.to_string(), account.to_string()));
        Ok(())
    }
}
