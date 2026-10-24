//! `app-core` — Microkernel foundation for the All-in-One Tauri 2.x Desktop App.
//!
//! Coordinates:
//! - **`AppModule` & `ModuleRegistry`**: Unified plugin contract for all 10 feature modules.
//! - **`EventBus`**: Lock-free clipboard suppression + async broadcast event bus, including
//!   Snipaste-style hierarchical region detection (`DetectedRegionRect`, `resolve_smart_region_at_point`).
//! - **`ShortcutNormalizer` & `ShortcutRegistry`**: Cross-platform hotkey mapping (`PrimaryMod`,
//!   `SecondaryMod`, `MetaMod`) for Windows and macOS with conflict detection.
//! - **`EnvProbeService`**: Zero-bundled-runtime environment detector (`ffmpeg`, `ffprobe`, `node`,
//!   `python3`, `cscript`, `powershell`, `osascript`, `zsh`) with macOS GUI `$PATH` hydration.
//! - **`DbPool` & `ConfigMigrationPipeline`**: Local SQLite WAL storage with automatic pre-upgrade
//!   backups, atomic incremental SQL migrations, and forward/backward-compatible JSON config migration.
//! - **`WindowPoolManager`**: 3-level WebView lifecycle manager guaranteeing 0 hidden WebViews on startup.

pub mod env_probe;
pub mod error;
pub mod event_bus;
pub mod module;
pub mod screenshot_region;
pub mod shortcut;
pub mod storage;
pub mod window_manager;

use std::{path::PathBuf, sync::Arc};
use parking_lot::RwLock;
use tracing::{info, warn};

pub use env_probe::{EnvProbeService, ExternalToolKind, ToolProbeResult};
pub use error::{CoreError, CoreResult};
pub use event_bus::{CaptureMode, EventBus, SystemEvent};
pub use module::{AppModule, ModuleContext, ModuleMetadata, ModuleRegistry};
pub use screenshot_region::{
    infer_visual_edge_box_rgba, resolve_smart_region_at_point, DetectedRegionRect,
    RegionHierarchyLevel, ResizeHandle, SelectionBox,
};
pub use shortcut::{RegisteredShortcut, ShortcutNormalizer, ShortcutRegistry, ShortcutSpec, TargetOs};
pub use storage::{
    AppConfig, AppliedMigrationRecord, ClipboardConfig, ConfigMigrationPipeline,
    CustomOnnxModelConfig, CustomToolPathsConfig, DbPool, GeneralConfig, ModuleMigration,
    OcrPriorityMode, PasswordVaultConfig, ScreenshotConfig, SharedConfig, UsageTrackingConfig,
    CURRENT_CONFIG_VERSION,
};
pub use window_manager::{
    ActiveWindowEntry, HeadlessWindowBackend, WindowBackend, WindowLifecyclePolicy,
    WindowPoolManager, WindowSpec,
};

/// Builder for assembling and bootstrapping the `AppKernel` daemon.
pub struct AppKernelBuilder {
    app_data_dir: PathBuf,
    in_memory_db: bool,
    target_os: TargetOs,
    window_backend: Arc<dyn WindowBackend>,
    registry: ModuleRegistry,
}

impl AppKernelBuilder {
    /// Creates a new kernel builder rooted at `app_data_dir`.
    pub fn new(app_data_dir: impl Into<PathBuf>) -> Self {
        Self {
            app_data_dir: app_data_dir.into(),
            in_memory_db: false,
            target_os: TargetOs::current(),
            window_backend: Arc::new(HeadlessWindowBackend),
            registry: ModuleRegistry::new(),
        }
    }

    /// Uses an in-memory SQLite database instead of `<app_data_dir>/app.db` (useful for tests).
    pub fn with_in_memory_db(mut self, in_memory: bool) -> Self {
        self.in_memory_db = in_memory;
        self
    }

    /// Overrides the target OS used for shortcut normalization.
    pub fn with_target_os(mut self, target_os: TargetOs) -> Self {
        self.target_os = target_os;
        self
    }

    /// Attaches a custom `WindowBackend` (such as the Tauri 2.x `AppHandle` adapter).
    pub fn with_window_backend(mut self, backend: Arc<dyn WindowBackend>) -> Self {
        self.window_backend = backend;
        self
    }

    /// Registers a feature module (`AppModule`) into the kernel.
    pub fn register_module<M: AppModule>(mut self, module: M) -> CoreResult<Self> {
        self.registry.register(module)?;
        Ok(self)
    }

    /// Bootstraps the microkernel in deterministic order:
    /// 1. Hydrates GUI `$PATH` on macOS.
    /// 2. Loads & migrates `config.json` via `ConfigMigrationPipeline`.
    /// 3. Opens `app.db` (WAL mode), backs up existing DB if migrations are pending, and applies
    ///    all core + module incremental SQL migrations.
    /// 4. Configures `EnvProbeService` with user custom tool paths.
    /// 5. Registers all module shortcuts (applying user overrides from `AppConfig`).
    /// 6. Invokes `on_init(&ctx)` across all registered modules (while creating 0 WebView windows).
    pub fn bootstrap(self) -> CoreResult<AppKernel> {
        // 1. Repair GUI PATH on macOS
        EnvProbeService::hydrate_gui_path_env();

        // 2. Load and migrate config.json
        std::fs::create_dir_all(&self.app_data_dir)?;
        let config_path = self.app_data_dir.join("config.json");
        let app_config = ConfigMigrationPipeline::load_or_init(&config_path)?;
        let shared_config: SharedConfig = Arc::new(RwLock::new(app_config.clone()));

        // 3. Open SQLite database and run core + module migrations
        let db = if self.in_memory_db {
            DbPool::open_in_memory()?
        } else {
            DbPool::open(self.app_data_dir.join("app.db"))?
        };

        let mut all_migrations = vec![ModuleMigration::new(
            "core",
            1,
            "Initialize core audit log and settings metadata table",
            "CREATE TABLE IF NOT EXISTS core_audit_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                module_id TEXT NOT NULL,
                event_name TEXT NOT NULL,
                detail TEXT,
                created_at INTEGER NOT NULL
            );",
        )];
        all_migrations.extend(self.registry.collect_all_migrations());
        let applied_migrations = db.run_migrations(&all_migrations)?;

        // 4. Configure EnvProbeService with custom tool overrides from config
        let env_probe = Arc::new(EnvProbeService::new());
        Self::sync_env_probe_overrides(&env_probe, &app_config.custom_tool_paths);

        // 5. Initialize ShortcutRegistry and register module default shortcuts + user overrides
        let shortcut_registry = Arc::new(ShortcutRegistry::new(self.target_os));
        for spec in self.registry.collect_all_default_shortcuts() {
            let user_override = app_config
                .shortcut_overrides
                .get(&spec.action_id)
                .map(String::as_str);
            if let Err(err) = shortcut_registry.register(spec, user_override) {
                warn!("Shortcut registration conflict or error (skipped): {}", err);
            }
        }

        // 6. Initialize WindowPoolManager (Level 0: 0 active WebView windows on startup)
        let window_pool = Arc::new(WindowPoolManager::new(self.window_backend));
        let event_bus = EventBus::default();

        let context = ModuleContext {
            app_data_dir: self.app_data_dir.clone(),
            db,
            event_bus,
            config: shared_config,
            env_probe,
            window_pool,
            shortcut_registry,
        };

        // 7. Execute `on_init` across all registered modules
        self.registry.init_all(&context)?;

        info!(
            modules = self.registry.len(),
            applied_migrations,
            active_webviews = context.window_pool.active_window_count(),
            "AppKernel bootstrap completed (Level 0 daemon ready)"
        );

        Ok(AppKernel {
            context,
            registry: Arc::new(self.registry),
            config_path,
        })
    }

    fn sync_env_probe_overrides(
        env_probe: &EnvProbeService,
        paths: &CustomToolPathsConfig,
    ) {
        env_probe.set_custom_override(ExternalToolKind::Ffmpeg, paths.ffmpeg_path.clone());
        env_probe.set_custom_override(ExternalToolKind::Ffprobe, paths.ffprobe_path.clone());
        env_probe.set_custom_override(ExternalToolKind::Node, paths.node_path.clone());
        env_probe.set_custom_override(ExternalToolKind::Python3, paths.python3_path.clone());
    }
}

/// Running microkernel instance holding the `ModuleContext` and `ModuleRegistry`.
pub struct AppKernel {
    context: ModuleContext,
    registry: Arc<ModuleRegistry>,
    config_path: PathBuf,
}

impl AppKernel {
    /// Returns a reference to the shared `ModuleContext`.
    pub fn context(&self) -> &ModuleContext {
        &self.context
    }

    /// Returns a reference to the `ModuleRegistry`.
    pub fn registry(&self) -> &ModuleRegistry {
        &self.registry
    }

    /// Triggers a shortcut by its `action_id` (e.g., `"screenshot:capture"`, `"clipboard:toggle_popup"`).
    pub fn trigger_action(&self, action_id: &str) -> CoreResult<()> {
        let reg = self
            .context
            .shortcut_registry
            .get_by_action(action_id)
            .ok_or_else(|| CoreError::ShortcutActionNotFound(action_id.to_string()))?;
        self.registry
            .dispatch_shortcut_action(&reg.spec.module_id, &reg.spec.action_id, &self.context)
    }

    /// Triggers a shortcut by its physical or logical key combination (e.g., `"F1"`, `"Ctrl+Shift+V"`).
    pub fn trigger_accelerator(&self, accelerator: &str) -> CoreResult<()> {
        let (module_id, action_id) = self
            .context
            .shortcut_registry
            .resolve_accelerator(accelerator)
            .ok_or_else(|| CoreError::ShortcutActionNotFound(accelerator.to_string()))?;
        self.registry
            .dispatch_shortcut_action(&module_id, &action_id, &self.context)
    }

    /// Updates and atomically persists `AppConfig`, synchronizing custom tool paths and broadcasting `ConfigReloaded`.
    pub fn update_config<F>(&self, mutator: F) -> CoreResult<()>
    where
        F: FnOnce(&mut AppConfig),
    {
        let updated = {
            let mut guard = self.context.config.write();
            mutator(&mut guard);
            guard.clone()
        };

        ConfigMigrationPipeline::save_atomic(&self.config_path, &updated)?;
        AppKernelBuilder::sync_env_probe_overrides(
            &self.context.env_probe,
            &updated.custom_tool_paths,
        );
        self.context.event_bus.publish(SystemEvent::ConfigReloaded);
        Ok(())
    }

    /// Gracefully shuts down all modules in reverse registration order and destroys all open WebViews.
    pub fn shutdown(&self) -> CoreResult<()> {
        let res = self.registry.shutdown_all(&self.context);
        let _ = self.context.window_pool.destroy_all();
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use serde_json::{json, Value};

    struct MockScreenshotModule {
        capture_triggers: Arc<AtomicUsize>,
    }

    impl AppModule for MockScreenshotModule {
        fn id(&self) -> &'static str {
            "screenshot"
        }

        fn name(&self) -> &'static str {
            "增强截图与贴图"
        }

        fn migrations(&self) -> Vec<ModuleMigration> {
            vec![ModuleMigration::new(
                "screenshot",
                1,
                "Create ocr_models_meta table",
                "CREATE TABLE IF NOT EXISTS ocr_models_meta (
                    id TEXT PRIMARY KEY,
                    model_name TEXT NOT NULL,
                    det_path TEXT NOT NULL,
                    rec_path TEXT NOT NULL
                );",
            )]
        }

        fn default_shortcuts(&self) -> Vec<ShortcutSpec> {
            vec![
                ShortcutSpec::new("screenshot", "screenshot:capture", "F1", "启动区域截图(支持手动框选与智能框推导)"),
                ShortcutSpec::new("screenshot", "screenshot:ocr", "PrimaryMod+Shift+O", "截图并立即 OCR"),
            ]
        }

        fn on_shortcut(&self, action_id: &str, ctx: &ModuleContext) -> CoreResult<()> {
            if action_id == "screenshot:capture" {
                self.capture_triggers.fetch_add(1, Ordering::SeqCst);
                let enable_smart = ctx.config.read().screenshot.enable_smart_region_detection;
                ctx.event_bus.publish(SystemEvent::ScreenCaptureRequested {
                    mode: CaptureMode::Standard,
                    enable_smart_region_detection: enable_smart,
                });
                ctx.window_pool
                    .open_window(WindowSpec::capture_overlay(0.0, 0.0, 1920.0, 1080.0))?;
            }
            Ok(())
        }
    }

    #[test]
    fn test_snipaste_smart_region_detection_and_wheel_hierarchy() {
        let regions = vec![
            DetectedRegionRect {
                region_id: 1,
                parent_id: None,
                level: RegionHierarchyLevel::Monitor,
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                label: Some("Primary Monitor".to_string()),
            },
            DetectedRegionRect {
                region_id: 2,
                parent_id: Some(1),
                level: RegionHierarchyLevel::TopLevelWindow,
                x: 100,
                y: 100,
                width: 800,
                height: 600,
                label: Some("VS Code".to_string()),
            },
            DetectedRegionRect {
                region_id: 3,
                parent_id: Some(2),
                level: RegionHierarchyLevel::ContainerPanel,
                x: 150,
                y: 150,
                width: 400,
                height: 300,
                label: Some("Editor Group".to_string()),
            },
            DetectedRegionRect {
                region_id: 4,
                parent_id: Some(3),
                level: RegionHierarchyLevel::UiElement,
                x: 180,
                y: 180,
                width: 120,
                height: 36,
                label: Some("Run Button".to_string()),
            },
        ];

        // Hover at (200, 190): should hit innermost UiElement ("Run Button", id=4)
        let hit0 = resolve_smart_region_at_point(&regions, 200, 190, 0).unwrap();
        assert_eq!(hit0.region_id, 4);

        // Scroll mouse wheel up 1 step: expands to ContainerPanel ("Editor Group", id=3)
        let hit1 = resolve_smart_region_at_point(&regions, 200, 190, 1).unwrap();
        assert_eq!(hit1.region_id, 3);

        // Scroll mouse wheel up 2 steps: expands to TopLevelWindow ("VS Code", id=2)
        let hit2 = resolve_smart_region_at_point(&regions, 200, 190, 2).unwrap();
        assert_eq!(hit2.region_id, 2);

        // Scroll mouse wheel up 3 steps: expands to Monitor (id=1)
        let hit3 = resolve_smart_region_at_point(&regions, 200, 190, 3).unwrap();
        assert_eq!(hit3.region_id, 1);
    }

    #[test]
    fn test_cross_platform_shortcut_normalization() {
        let win_accel =
            ShortcutNormalizer::normalize_for_os("PrimaryMod+Shift+V", TargetOs::Windows).unwrap();
        assert_eq!(win_accel, "Ctrl+Shift+V");

        let mac_accel =
            ShortcutNormalizer::normalize_for_os("PrimaryMod+Shift+V", TargetOs::MacOs).unwrap();
        assert_eq!(mac_accel, "Shift+Super+V");

        let mac_badge =
            ShortcutNormalizer::format_display_label("PrimaryMod+Shift+V", TargetOs::MacOs)
                .unwrap();
        assert_eq!(mac_badge, "⇧ + ⌘ + V");
    }

    #[test]
    fn test_config_migration_v1_to_v2_preserves_unknown_fields() {
        let legacy_v1 = json!({
            "config_version": 1,
            "max_clipboard_items": 150,
            "future_plugin_flag": "keep_me"
        });

        let migrated = ConfigMigrationPipeline::migrate_value(legacy_v1, 1).unwrap();
        let cfg: AppConfig = serde_json::from_value(migrated).unwrap();

        assert_eq!(cfg.config_version, CURRENT_CONFIG_VERSION);
        assert_eq!(cfg.clipboard.max_history_items, 150);
        assert!(cfg.screenshot.enable_manual_box_selection);
        assert!(cfg.screenshot.enable_smart_region_detection);
        assert_eq!(
            cfg.unknown_fields.get("future_plugin_flag"),
            Some(&Value::String("keep_me".to_string()))
        );
    }

    #[test]
    fn test_kernel_bootstrap_zero_webview_and_shortcut_dispatch() {
        let temp_dir = tempfile::tempdir().unwrap();
        let triggers = Arc::new(AtomicUsize::new(0));

        let kernel = AppKernelBuilder::new(temp_dir.path())
            .with_target_os(TargetOs::MacOs)
            .register_module(MockScreenshotModule {
                capture_triggers: Arc::clone(&triggers),
            })
            .unwrap()
            .bootstrap()
            .unwrap();

        // Level 0 guarantee: 0 WebView windows open after daemon startup
        assert_eq!(kernel.context().window_pool.active_window_count(), 0);

        // Trigger F1 screenshot shortcut
        kernel.trigger_accelerator("F1").unwrap();
        assert_eq!(triggers.load(Ordering::SeqCst), 1);
        assert_eq!(kernel.context().window_pool.active_window_count(), 1);

        // Destroy transient overlay after capture completes -> back to 0 WebViews
        kernel
            .context()
            .window_pool
            .destroy_window("capture-overlay")
            .unwrap();
        assert_eq!(kernel.context().window_pool.active_window_count(), 0);

        kernel.shutdown().unwrap();
    }
}
