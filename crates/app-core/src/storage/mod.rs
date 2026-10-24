//! Local persistence subsystem: SQLite storage engine (`db`) and versioned configuration (`config`).

pub mod config;
pub mod db;

pub use config::{
    AppConfig, ClipboardConfig, ConfigMigrationPipeline, CustomOnnxModelConfig,
    CustomToolPathsConfig, GeneralConfig, OcrPriorityMode, PasswordVaultConfig,
    ScreenshotConfig, SharedConfig, UsageTrackingConfig, CURRENT_CONFIG_VERSION,
};
pub use db::{AppliedMigrationRecord, DbPool, ModuleMigration};
