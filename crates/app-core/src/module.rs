//! Plugin architecture contract (`AppModule` Trait), `ModuleContext` dependency injection container,
//! and `ModuleRegistry` lifecycle orchestrator for all 10 feature modules.

use crate::{
    env_probe::EnvProbeService,
    error::{CoreError, CoreResult},
    event_bus::EventBus,
    shortcut::{ShortcutRegistry, ShortcutSpec},
    storage::{DbPool, ModuleMigration, SharedConfig},
    window_manager::WindowPoolManager,
};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tracing::{error, info};

/// Shared runtime context injected into every `AppModule` hook.
#[derive(Clone)]
pub struct ModuleContext {
    /// Application data directory on disk (`app.db`, `config.json`, `clipboard_blobs/`, `backups/`).
    pub app_data_dir: PathBuf,
    /// Thread-safe SQLite connection pool (WAL mode).
    pub db: DbPool,
    /// Publish-subscribe event bus for inter-module signaling.
    pub event_bus: EventBus,
    /// Versioned, hot-reloadable configuration (`Arc<RwLock<AppConfig>>`).
    pub config: SharedConfig,
    /// Zero-bundled-runtime system `PATH` detector (`ffmpeg`, `node`, `python3`, etc.).
    pub env_probe: Arc<EnvProbeService>,
    /// On-demand WebView lifecycle manager enforcing zero hidden background WebViews.
    pub window_pool: Arc<WindowPoolManager>,
    /// Global cross-platform shortcut normalizer and conflict-aware registry.
    pub shortcut_registry: Arc<ShortcutRegistry>,
}

impl ModuleContext {
    /// Feature crates own their setting types; the kernel need not import them.
    pub fn module_settings<T: serde::de::DeserializeOwned + Default>(
        &self,
        id: &str,
    ) -> CoreResult<T> {
        self.config
            .read()
            .modules
            .settings
            .get(id)
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map(|value| value.unwrap_or_default())
            .map_err(CoreError::from)
    }
}

/// Metadata summary of a registered feature module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleMetadata {
    pub id: String,
    pub name: String,
    pub shortcut_count: usize,
    pub migration_count: usize,
    pub enabled: bool,
}

/// Standard plugin contract that all 10 feature modules (`mod-screenshot`, `mod-clipboard`,
/// `mod-screen-ink`, `mod-sticky-note`, `mod-app-usage`, `mod-script-runner`, `mod-translator`,
/// `mod-media-proc`, `mod-window-pin`, `mod-password-vault`) must implement.
pub trait AppModule: Send + Sync + 'static {
    /// Unique machine identifier for the module (e.g., `"screenshot"`, `"clipboard"`).
    fn id(&self) -> &'static str;

    /// Human-readable display name (e.g., `"增强截图与贴图"`, `"剪切板管理器"`).
    fn name(&self) -> &'static str;

    /// Returns ordered SQLite schema migrations owned by this module.
    fn migrations(&self) -> Vec<ModuleMigration> {
        Vec::new()
    }

    /// Declares the default cross-platform logical shortcuts for this module.
    fn default_shortcuts(&self) -> Vec<ShortcutSpec> {
        Vec::new()
    }

    /// Invoked once during daemon startup after SQLite migrations and shortcut registrations complete.
    /// Resident background modules (`mod-clipboard`, `mod-app-usage`) spawn their lightweight tasks here;
    /// on-demand modules (`mod-screenshot`, `mod-media-proc`) remain zero-memory no-ops until triggered.
    fn on_init(&self, _ctx: &ModuleContext) -> CoreResult<()> {
        Ok(())
    }

    /// Invoked when any shortcut belonging to this module is triggered.
    fn on_shortcut(&self, _action_id: &str, _ctx: &ModuleContext) -> CoreResult<()> {
        Ok(())
    }

    /// Invoked in reverse registration order before the daemon exits or sleeps.
    /// Critical for restoring external topmost windows (`mod-window-pin`), flushing usage timers
    /// (`mod-app-usage`), terminating child processes (`mod-media-proc`, `mod-script-runner`),
    /// and zeroizing master keys (`mod-password-vault`).
    fn on_shutdown(&self, _ctx: &ModuleContext) -> CoreResult<()> {
        Ok(())
    }
}

/// Container that manages registration, initialization, shortcut routing, and reverse-order shutdown
/// for all feature modules.
#[derive(Default)]
pub struct ModuleRegistry {
    modules_ordered: Vec<Arc<dyn AppModule>>,
    by_id: HashMap<String, Arc<dyn AppModule>>,
    enabled: RwLock<HashSet<String>>,
    transition: Mutex<()>,
}

impl ModuleRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a feature module into the microkernel.
    /// Returns `CoreError::DuplicateModule` if a module with the same `id()` is already mounted.
    pub fn register<M: AppModule>(&mut self, module: M) -> CoreResult<()> {
        self.register_arc(Arc::new(module))
    }

    /// Registers an already `Arc`-wrapped feature module.
    pub fn register_arc(&mut self, module: Arc<dyn AppModule>) -> CoreResult<()> {
        let id = module.id().to_string();
        if self.by_id.contains_key(&id) {
            return Err(CoreError::DuplicateModule(id));
        }
        self.enabled.get_mut().insert(id.clone());
        self.by_id.insert(id, Arc::clone(&module));
        self.modules_ordered.push(module);
        Ok(())
    }

    /// Collects all SQLite schema migrations across all registered modules.
    pub fn collect_all_migrations(&self) -> Vec<ModuleMigration> {
        let mut all = Vec::new();
        for module in &self.modules_ordered {
            all.extend(module.migrations());
        }
        all
    }

    /// Collects all default shortcut specifications across all registered modules.
    pub fn collect_all_default_shortcuts(&self) -> Vec<ShortcutSpec> {
        let mut all = Vec::new();
        for module in &self.modules_ordered {
            if self.is_enabled(module.id()) {
                all.extend(module.default_shortcuts());
            }
        }
        all
    }

    /// Executes `on_init` on each registered module in registration order.
    pub fn init_all(&self, ctx: &ModuleContext) -> CoreResult<()> {
        let mut initialized: Vec<&Arc<dyn AppModule>> = Vec::new();
        for module in &self.modules_ordered {
            if !self.is_enabled(module.id()) {
                continue;
            }
            if let Err(err) = module.on_init(ctx) {
                let _ = module.on_shutdown(ctx);
                for previous in initialized.iter().rev() {
                    let _ = previous.on_shutdown(ctx);
                }
                return Err(CoreError::ModuleLifecycle {
                    module_id: module.id().to_string(),
                    hook: "on_init",
                    reason: err.to_string(),
                });
            }
            initialized.push(module);
            info!(
                module_id = module.id(),
                module_name = module.name(),
                "Initialized module"
            );
        }
        Ok(())
    }

    /// Routes a shortcut `action_id` directly to its owning module's `on_shortcut` handler.
    pub fn dispatch_shortcut_action(
        &self,
        module_id: &str,
        action_id: &str,
        ctx: &ModuleContext,
    ) -> CoreResult<()> {
        let module = self
            .by_id
            .get(module_id)
            .ok_or_else(|| CoreError::ModuleNotFound(module_id.to_string()))?;

        self.with_enabled(module_id, || module.on_shortcut(action_id, ctx))
            .map_err(|err| CoreError::ModuleLifecycle {
                module_id: module_id.to_string(),
                hook: "on_shortcut",
                reason: err.to_string(),
            })
    }

    /// Executes `on_shutdown` on all registered modules in **reverse** registration order.
    /// Continues shutting down remaining modules even if an individual module logs an error,
    /// returning the first encountered error (if any) after all modules have had a chance to clean up.
    pub fn shutdown_all(&self, ctx: &ModuleContext) -> CoreResult<()> {
        let mut first_err: Option<CoreError> = None;

        let _transition = self.transition.lock();
        // 先等待正在执行的操作结束，再在不持有活动状态锁的情况下执行清理钩子。
        let enabled = std::mem::take(&mut *self.enabled.write());
        for module in self.modules_ordered.iter().rev() {
            if !enabled.contains(module.id()) {
                continue;
            }
            if let Err(err) = module.on_shutdown(ctx) {
                error!(
                    module_id = module.id(),
                    error = %err,
                    "Module on_shutdown hook reported an error"
                );
                if first_err.is_none() {
                    first_err = Some(CoreError::ModuleLifecycle {
                        module_id: module.id().to_string(),
                        hook: "on_shutdown",
                        reason: err.to_string(),
                    });
                }
            } else {
                info!(module_id = module.id(), "Module shut down cleanly");
            }
        }

        for shortcut in ctx.shortcut_registry.list_all() {
            ctx.shortcut_registry.unregister(&shortcut.spec.action_id);
        }
        match first_err {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    /// Returns metadata for all registered modules in registration order.
    pub fn list_modules(&self) -> Vec<ModuleMetadata> {
        self.modules_ordered
            .iter()
            .map(|m| ModuleMetadata {
                id: m.id().to_string(),
                name: m.name().to_string(),
                shortcut_count: m.default_shortcuts().len(),
                migration_count: m.migrations().len(),
                enabled: self.is_enabled(m.id()),
            })
            .collect()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.by_id.contains_key(id)
    }
    pub fn is_enabled(&self, id: &str) -> bool {
        self.enabled.read().contains(id)
    }

    pub(crate) fn configure_enabled(&self, config: &crate::ModulesConfig) {
        *self.enabled.write() = self
            .by_id
            .keys()
            .filter(|id| config.is_enabled(id))
            .cloned()
            .collect();
    }

    /// Hold an activity lease so disabling waits for in-flight IPC/background work.
    pub fn with_enabled<T>(
        &self,
        id: &str,
        action: impl FnOnce() -> CoreResult<T>,
    ) -> CoreResult<T> {
        if !self.contains(id) {
            return Err(CoreError::ModuleNotFound(id.to_string()));
        }
        let enabled = self.enabled.read();
        if !enabled.contains(id) {
            return Err(CoreError::ModuleDisabled(id.to_string()));
        }
        action()
    }

    pub(crate) fn set_enabled(
        &self,
        id: &str,
        enable: bool,
        ctx: &ModuleContext,
        persist: impl FnOnce() -> CoreResult<()>,
    ) -> CoreResult<()> {
        let module = self
            .by_id
            .get(id)
            .ok_or_else(|| CoreError::ModuleNotFound(id.to_string()))?;
        let _transition = self.transition.lock();
        if self.is_enabled(id) == enable {
            return Ok(());
        }
        let specs = module.default_shortcuts();
        if enable {
            let config = ctx.config.read().clone();
            let mut registered: Vec<String> = Vec::new();
            for spec in &specs {
                if let Err(error) = ctx.shortcut_registry.register(
                    spec.clone(),
                    config
                        .shortcut_overrides
                        .get(&spec.action_id)
                        .map(String::as_str),
                ) {
                    for action in registered {
                        ctx.shortcut_registry.unregister(&action);
                    }
                    return Err(error);
                }
                registered.push(spec.action_id.clone());
            }
            if let Err(error) = module.on_init(ctx).and_then(|_| persist()) {
                let _ = module.on_shutdown(ctx);
                for action in registered {
                    ctx.shortcut_registry.unregister(&action);
                }
                return Err(error);
            }
            self.enabled.write().insert(id.to_string());
        } else {
            // 获取写锁会等待已有 IPC/后台操作结束，再阻止新操作进入。
            self.enabled.write().remove(id);
            if let Err(error) = module.on_shutdown(ctx) {
                self.enabled.write().insert(id.to_string());
                return Err(error);
            }
            if let Err(error) = persist() {
                let restarted = module.on_init(ctx);
                if restarted.is_ok() {
                    self.enabled.write().insert(id.to_string());
                }
                return Err(error);
            }
            for spec in specs {
                ctx.shortcut_registry.unregister(&spec.action_id);
            }
        }
        Ok(())
    }

    /// Returns the number of registered modules.
    pub fn len(&self) -> usize {
        self.modules_ordered.len()
    }

    pub fn is_empty(&self) -> bool {
        self.modules_ordered.is_empty()
    }
}
