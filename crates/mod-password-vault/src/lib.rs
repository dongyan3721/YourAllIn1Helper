//! 功能 10 M1：主密码加密密码本。
//!
//! 未通过独立安全审查前不挂载 Host，不处理真实秘密。生物识别与秘密复制
//! 在本版本明确禁用；模块不依赖 EventBus 或剪贴板模块。

mod crypto;
mod error;
mod model;
pub mod native;
mod service;
mod store;

#[cfg(test)]
#[path = "../tests/unit/lib.rs"]
mod unit_tests;

pub use error::{VaultError, VaultResult};
pub use model::{
    LockReason, PasswordPolicy, VaultEntry, VaultEntryInput, VaultEntryPage, VaultEntrySummary,
    VaultEvent, VaultMutationResult, VaultStatus, AUTO_LOCK_SECONDS, MODULE_ID, REVEAL_SECONDS,
};
pub use service::{VaultEventSink, VaultService};

use app_core::{AppModule, CoreError, CoreResult, ModuleContext, ModuleMigration};
use os_hal::ActiveAppWatcher;
use parking_lot::Mutex;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tracing::warn;

pub struct PasswordVaultModule {
    service: Arc<VaultService>,
    watcher: Arc<dyn ActiveAppWatcher>,
    worker: Mutex<Option<JoinHandle<()>>>,
    stop: Arc<AtomicBool>,
}

impl PasswordVaultModule {
    pub fn new(watcher: Arc<dyn ActiveAppWatcher>) -> Self {
        Self::with_service(watcher, Arc::new(VaultService::new()))
    }

    #[doc(hidden)]
    pub fn with_service(watcher: Arc<dyn ActiveAppWatcher>, service: Arc<VaultService>) -> Self {
        Self {
            service,
            watcher,
            worker: Mutex::new(None),
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn service(&self) -> Arc<VaultService> {
        Arc::clone(&self.service)
    }

    /// 后台与显式测试共用同一只读采样入口，绝不刷新系统空闲时间。
    pub fn sample_now(&self) {
        let status = self.service.status();
        if !status.initialized || status.locked {
            return;
        }
        match self.watcher.is_screen_locked() {
            Ok(true) => {
                self.service.lock(LockReason::ScreenLock);
                return;
            }
            Ok(false) => {}
            Err(error) => {
                self.service
                    .record_capability_result(Some(error.to_string()));
                self.service.lock(LockReason::CapabilityError);
                return;
            }
        }
        match self.watcher.system_idle_seconds() {
            Ok(idle) if idle >= AUTO_LOCK_SECONDS => {
                self.service.lock(LockReason::Idle);
            }
            Ok(_) => self.service.record_capability_result(None),
            Err(error) => {
                self.service
                    .record_capability_result(Some(error.to_string()));
                self.service.lock(LockReason::CapabilityError);
            }
        }
    }

    /// 原生锁屏/会话离开通知的窄接入口；不包含任何秘密载荷。
    pub fn notify_screen_lock(&self) {
        self.service.lock(LockReason::ScreenLock);
    }

    /// 原生睡眠/电源通知的窄接入口；唤醒采样发现长时间跳变也走此路径。
    pub fn notify_sleep(&self) {
        self.service.lock(LockReason::Sleep);
    }

    /// 页面离开、pagehide、窗口隐藏/关闭统一进入后端锁定。
    pub fn notify_page_close(&self) {
        self.service.lock(LockReason::PageClose);
    }

    fn start_worker(&self) -> CoreResult<()> {
        if self.worker.lock().is_some() {
            return Ok(());
        }
        self.stop.store(false, Ordering::SeqCst);
        let service = Arc::clone(&self.service);
        let watcher = Arc::clone(&self.watcher);
        let stop = Arc::clone(&self.stop);
        let handle = thread::Builder::new()
            .name("password-vault-lock-watch".to_string())
            .spawn(move || {
                let mut previous_tick = Instant::now();
                while !stop.load(Ordering::SeqCst) {
                    thread::sleep(Duration::from_secs(1));
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let now = Instant::now();
                    if now.duration_since(previous_tick) > Duration::from_secs(3) {
                        service.lock(LockReason::Sleep);
                    }
                    previous_tick = now;
                    let status = service.status();
                    if !status.initialized || status.locked {
                        continue;
                    }
                    match watcher.is_screen_locked() {
                        Ok(true) => {
                            service.lock(LockReason::ScreenLock);
                            continue;
                        }
                        Ok(false) => {}
                        Err(error) => {
                            service.record_capability_result(Some(error.to_string()));
                            service.lock(LockReason::CapabilityError);
                            continue;
                        }
                    }
                    match watcher.system_idle_seconds() {
                        Ok(idle) if idle >= AUTO_LOCK_SECONDS => {
                            service.lock(LockReason::Idle);
                        }
                        Ok(_) => service.record_capability_result(None),
                        Err(error) => {
                            service.record_capability_result(Some(error.to_string()));
                            service.lock(LockReason::CapabilityError);
                        }
                    }
                }
            })
            .map_err(CoreError::Io)?;
        *self.worker.lock() = Some(handle);
        Ok(())
    }

    fn stop_worker(&self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.worker.lock().take() {
            if handle.join().is_err() {
                warn!("Password vault lock worker terminated unexpectedly");
            }
        }
    }
}

impl Drop for PasswordVaultModule {
    fn drop(&mut self) {
        self.service.set_accepting(false);
        self.stop_worker();
        self.service.lock(LockReason::Shutdown);
    }
}

impl AppModule for PasswordVaultModule {
    fn id(&self) -> &'static str {
        MODULE_ID
    }

    fn name(&self) -> &'static str {
        "生物识别加密密码本（M1 主密码）"
    }

    fn migrations(&self) -> Vec<ModuleMigration> {
        store::migrations()
    }

    fn on_init(&self, ctx: &ModuleContext) -> CoreResult<()> {
        self.service
            .refresh_from_disk(ctx)
            .map_err(|error| CoreError::Config(error.to_string()))?;
        self.service.set_accepting(true);
        if let Err(error) = self.start_worker() {
            self.service.set_accepting(false);
            self.service.lock(LockReason::CapabilityError);
            return Err(error);
        }
        Ok(())
    }

    fn on_shutdown(&self, _ctx: &ModuleContext) -> CoreResult<()> {
        self.service.set_accepting(false);
        self.stop_worker();
        self.service.lock(LockReason::ModuleDisabled);
        Ok(())
    }
}
