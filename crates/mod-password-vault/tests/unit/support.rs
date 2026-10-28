#![allow(dead_code)]

use crate::{
    crypto::VaultRandom, PasswordVaultModule, SecretString, VaultEntryInput, VaultError,
    VaultResult, VaultService,
};
use app_core::{AppKernel, AppKernelBuilder};
use os_hal::{ActiveAppSnapshot, ActiveAppWatcher, HalError, HalResult};
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};
use tempfile::TempDir;
use zeroize::Zeroizing;

pub const MASTER: &str = "correct-horse-vault-2026";

pub fn input(marker: &str) -> VaultEntryInput {
    VaultEntryInput {
        title: SecretString::new(format!("虚构站点 {marker}")),
        url: SecretString::new(format!("https://{marker}.example.test/login")),
        category: SecretString::from("测试分类"),
        username: SecretString::new(format!("qa-{marker}@example.test")),
        password: SecretString::new(format!("not-a-real-password-{marker}")),
        totp_secret: SecretString::new(format!("FAKE-TOTP-{marker}")),
        notes: SecretString::new(format!("仅用于自动化测试的备注 {marker}")),
        archived: false,
    }
}

#[derive(Debug)]
pub struct CountingRandom {
    calls: AtomicU64,
    fail_at: Option<u64>,
    repeat_at: Option<(u64, u64)>,
    values: Mutex<HashMap<u64, Vec<u8>>>,
}

impl CountingRandom {
    pub fn new() -> Self {
        Self {
            calls: AtomicU64::new(0),
            fail_at: None,
            repeat_at: None,
            values: Mutex::new(HashMap::new()),
        }
    }

    pub fn failing_at(call: u64) -> Self {
        Self {
            fail_at: Some(call),
            ..Self::new()
        }
    }

    pub fn repeating(call: u64, source: u64) -> Self {
        Self {
            repeat_at: Some((call, source)),
            ..Self::new()
        }
    }
}

impl VaultRandom for CountingRandom {
    fn fill(&self, destination: &mut [u8]) -> VaultResult<()> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if self.fail_at == Some(call) {
            return Err(VaultError::RandomSourceFailed);
        }
        let bytes = if let Some((repeat, source)) = self.repeat_at {
            if repeat == call {
                self.values
                    .lock()
                    .get(&source)
                    .cloned()
                    .unwrap_or_else(|| vec![0; destination.len()])
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        if bytes.len() == destination.len() && !bytes.is_empty() {
            destination.copy_from_slice(&bytes);
        } else {
            for (index, byte) in destination.iter_mut().enumerate() {
                *byte = ((call * 37 + index as u64 * 19) % 251 + 1) as u8;
            }
        }
        self.values.lock().insert(call, destination.to_vec());
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct FakeWatcher {
    pub idle: AtomicU64,
    pub locked: AtomicBool,
    pub fail: AtomicBool,
}

impl ActiveAppWatcher for FakeWatcher {
    fn current_active_app(&self) -> HalResult<ActiveAppSnapshot> {
        Err(HalError::Unsupported("not_needed_by_vault"))
    }

    fn system_idle_seconds(&self) -> HalResult<u64> {
        if self.fail.load(Ordering::SeqCst) {
            Err(HalError::Unsupported("test_idle_failure"))
        } else {
            Ok(self.idle.load(Ordering::SeqCst))
        }
    }

    fn is_screen_locked(&self) -> HalResult<bool> {
        if self.fail.load(Ordering::SeqCst) {
            Err(HalError::NativeCallFailed("test lock failure".to_string()))
        } else {
            Ok(self.locked.load(Ordering::SeqCst))
        }
    }
}

pub struct Fixture {
    pub _temp: TempDir,
    pub kernel: AppKernel,
    pub module: Arc<PasswordVaultModule>,
    pub service: Arc<VaultService>,
    pub watcher: Arc<FakeWatcher>,
}

impl Fixture {
    pub fn new() -> Self {
        Self::with_random(Arc::new(CountingRandom::new()))
    }

    pub fn with_random(random: Arc<dyn VaultRandom>) -> Self {
        let temp = TempDir::new().expect("temporary vault directory");
        Self::build(temp, None, random, None)
    }

    pub fn with_keystore(keystore: Arc<dyn crate::keystore::VaultKeyStore>) -> Self {
        let temp = TempDir::new().expect("temporary vault directory");
        Self::build(temp, None, Arc::new(CountingRandom::new()), Some(keystore))
    }

    pub fn reopen(&self) -> Self {
        let temp = TempDir::new().expect("temporary marker directory");
        Self::build(
            temp,
            Some(self._temp.path().to_path_buf()),
            Arc::new(CountingRandom::new()),
            None,
        )
    }

    fn build(
        temp: TempDir,
        existing_path: Option<std::path::PathBuf>,
        random: Arc<dyn VaultRandom>,
        keystore: Option<Arc<dyn crate::keystore::VaultKeyStore>>,
    ) -> Self {
        let watcher = Arc::new(FakeWatcher::default());
        let service = Arc::new(match keystore {
            Some(keystore) => VaultService::with_parts(random, keystore),
            None => VaultService::with_random(random),
        });
        let module = Arc::new(PasswordVaultModule::with_service(
            watcher.clone(),
            service.clone(),
        ));
        let path = existing_path.unwrap_or_else(|| temp.path().to_path_buf());
        let kernel = AppKernelBuilder::new(path)
            .register_module_arc(module.clone())
            .expect("register vault")
            .bootstrap()
            .expect("bootstrap vault");
        // 并行 Argon2 测试可能让线程调度超过三秒并触发保守睡眠锁定；
        // 普通服务测试改为显式 sample_now，避免把 CPU 饱和当成非确定性断言。
        module.stop_worker();
        Self {
            _temp: temp,
            kernel,
            module,
            service,
            watcher,
        }
    }

    pub async fn initialized() -> Self {
        let fixture = Self::new();
        fixture.initialize().await;
        fixture
    }

    pub async fn unlocked() -> Self {
        let fixture = Self::initialized().await;
        fixture.unlock().await;
        fixture
    }

    pub async fn initialize(&self) {
        self.service
            .initialize(
                self.kernel.context().clone(),
                Zeroizing::new(MASTER.to_string()),
                Zeroizing::new(MASTER.to_string()),
            )
            .await
            .expect("initialize fictional vault");
    }

    pub async fn unlock(&self) {
        self.service
            .unlock_master_password(
                self.kernel.context().clone(),
                Zeroizing::new(MASTER.to_string()),
            )
            .await
            .expect("unlock fictional vault");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.kernel.shutdown();
    }
}
