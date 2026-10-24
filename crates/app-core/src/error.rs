//! Unified error types and result alias for the `app-core` microkernel.

use thiserror::Error;

/// Standard result alias used across `app-core` and feature modules.
pub type CoreResult<T> = Result<T, CoreError>;

/// Categorized errors produced by the microkernel infrastructure.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("Database operation failed: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("Database schema migration failed at v{version} ({module_id}): {reason}")]
    Migration {
        version: u32,
        module_id: String,
        reason: String,
    },

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Shortcut conflict detected for '{hotkey}': action '{new_action}' conflicts with existing action '{existing_action}' (module '{existing_module}')")]
    ShortcutConflict {
        hotkey: String,
        new_action: String,
        existing_action: String,
        existing_module: String,
    },

    #[error("Invalid shortcut expression '{hotkey}': {reason}")]
    InvalidShortcut { hotkey: String, reason: String },

    #[error("Shortcut action '{0}' is not registered")]
    ShortcutActionNotFound(String),

    #[error("Module '{0}' is already registered in ModuleRegistry")]
    DuplicateModule(String),

    #[error("Module '{0}' not found in ModuleRegistry")]
    ModuleNotFound(String),

    #[error("Module '{module_id}' lifecycle hook '{hook}' failed: {reason}")]
    ModuleLifecycle {
        module_id: String,
        hook: &'static str,
        reason: String,
    },

    #[error("Window operation failed for label '{label}': {reason}")]
    WindowLifecycle { label: String, reason: String },

    #[error("External tool '{tool}' is not available in system PATH or custom configuration: {hint}")]
    ExternalToolUnavailable { tool: String, hint: String },

    #[error("Internal kernel error: {0}")]
    Internal(#[from] anyhow::Error),
}
