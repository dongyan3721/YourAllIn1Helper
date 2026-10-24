//! 配置文件加载、版本迁移与原子保存。
use super::schema::*;
use crate::{CoreError, CoreResult};
use chrono::Utc;
use serde_json::{Map, Value};
use std::{fs, path::Path};
use tracing::info;

/// Chained configuration migrator that upgrades legacy `config.json` files to `CURRENT_CONFIG_VERSION`.
pub struct ConfigMigrationPipeline;

impl ConfigMigrationPipeline {
    /// Loads `AppConfig` from `config_path`, automatically backing up and migrating older schema versions,
    /// or creating a default `config.json` if the file does not exist yet.
    pub fn load_or_init(config_path: impl AsRef<Path>) -> CoreResult<AppConfig> {
        let path = config_path.as_ref();
        if !path.exists() {
            let default_cfg = AppConfig::default();
            Self::save_atomic(path, &default_cfg)?;
            return Ok(default_cfg);
        }

        let raw_text = fs::read_to_string(path)?;
        let raw_value: Value = serde_json::from_str(&raw_text)?;

        let detected_version = raw_value
            .get("config_version")
            .and_then(Value::as_u64)
            .unwrap_or(1) as u32;

        if detected_version < CURRENT_CONFIG_VERSION {
            // Backup legacy config file before transforming
            let ts = Utc::now().format("%Y%m%d_%H%M%S");
            let backup_path = path.with_extension(format!("json.v{}.{}.bak", detected_version, ts));
            let _ = fs::copy(path, &backup_path);

            let migrated_val = Self::migrate_value(raw_value, detected_version)?;
            let config: AppConfig = serde_json::from_value(migrated_val)?;
            Self::save_atomic(path, &config)?;
            info!(
                from_version = detected_version,
                to_version = CURRENT_CONFIG_VERSION,
                "Migrated config.json to latest schema version"
            );
            Ok(config)
        } else {
            let config: AppConfig = serde_json::from_value(raw_value)?;
            Ok(config)
        }
    }

    /// Transforms a raw JSON configuration value step-by-step from `from_version` up to `CURRENT_CONFIG_VERSION`.
    pub fn migrate_value(mut value: Value, mut current_version: u32) -> CoreResult<Value> {
        let obj = value
            .as_object_mut()
            .ok_or_else(|| CoreError::Config("Root config JSON must be an object".to_string()))?;

        while current_version < CURRENT_CONFIG_VERSION {
            match current_version {
                1 => {
                    // v1 -> v2 migration:
                    // 1) Move legacy flat `max_clipboard_items` into `clipboard.max_history_items`
                    if let Some(legacy_max) = obj.remove("max_clipboard_items") {
                        let clip_entry = obj
                            .entry("clipboard")
                            .or_insert_with(|| Value::Object(Map::new()));
                        if let Some(clip_obj) = clip_entry.as_object_mut() {
                            clip_obj.entry("max_history_items").or_insert(legacy_max);
                        }
                    }

                    // 2) Ensure `screenshot.enable_smart_region_detection` & `enable_manual_box_selection` exist
                    let shot_entry = obj
                        .entry("screenshot")
                        .or_insert_with(|| Value::Object(Map::new()));
                    if let Some(shot_obj) = shot_entry.as_object_mut() {
                        shot_obj
                            .entry("enable_manual_box_selection")
                            .or_insert(Value::Bool(true));
                        shot_obj
                            .entry("enable_smart_region_detection")
                            .or_insert(Value::Bool(true));
                        shot_obj
                            .entry("smart_region_wheel_expand")
                            .or_insert(Value::Bool(true));
                    }

                    current_version = 2;
                    obj.insert("config_version".to_string(), Value::from(current_version));
                }
                2 => {
                    // Existing feature settings and unknown fields are preserved.
                    obj.entry("modules")
                        .or_insert(serde_json::to_value(ModulesConfig::default())?);
                    current_version = 3;
                    obj.insert("config_version".to_string(), Value::from(current_version));
                }
                _ => {
                    current_version = CURRENT_CONFIG_VERSION;
                    obj.insert("config_version".to_string(), Value::from(current_version));
                }
            }
        }

        Ok(value)
    }

    /// Saves `AppConfig` atomically using a temporary file and rename to prevent corruption on power loss.
    pub fn save_atomic(config_path: impl AsRef<Path>, config: &AppConfig) -> CoreResult<()> {
        let path = config_path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }

        let tmp_path = path.with_extension("json.tmp");
        let serialized = serde_json::to_string_pretty(config)?;
        fs::write(&tmp_path, serialized)?;
        fs::rename(&tmp_path, path)?;
        Ok(())
    }
}
