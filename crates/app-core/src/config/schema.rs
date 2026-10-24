//! 配置结构与缺省值；不引用具体业务模块。
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{collections::HashMap, path::PathBuf, sync::Arc};

/// Current schema version of `config.json`.
pub const CURRENT_CONFIG_VERSION: u32 = 3;

/// Thread-safe shared configuration handle injected into `ModuleContext`.
pub type SharedConfig = Arc<RwLock<AppConfig>>;

/// Priority mode for OCR inference in Module 01 (`mod-screenshot`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OcrPriorityMode {
    /// Prefer system native OCR (`Windows.Media.Ocr` / macOS `Vision`), fallback to ONNX if requested.
    SystemNativeFirst,
    /// Prefer user-configured custom ONNX model weights (`ort` GPU -> NPU -> CPU fallback chain).
    CustomModelFirst,
}

impl Default for OcrPriorityMode {
    fn default() -> Self {
        Self::SystemNativeFirst
    }
}

/// User-configured custom ONNX model paths and hardware acceleration preferences.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomOnnxModelConfig {
    pub det_model_path: Option<PathBuf>,
    pub rec_model_path: Option<PathBuf>,
    pub keys_dict_path: Option<PathBuf>,
    /// Hardware execution provider preference: `["gpu", "npu", "cpu"]`.
    pub hardware_fallback_order: Vec<String>,
}

/// Configuration for Module 01: Screenshot, Snipaste-style smart region detection, Pin, OCR, Scroll & GIF.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenshotConfig {
    /// Enable manual freehand drag box selection after pressing capture shortcut.
    pub enable_manual_box_selection: bool,
    /// Enable Snipaste-like bounding-box smart region detection (Window/SubWindow + UIA/AXUIElement + visual edge box).
    pub enable_smart_region_detection: bool,
    /// Allow mouse wheel / `Tab` to expand or shrink detected bounding box across parent-child UI hierarchy.
    pub smart_region_wheel_expand: bool,
    /// Show RGB/HEX pixel magnifier crosshair during selection.
    pub show_magnifier: bool,
    /// OCR engine priority strategy.
    pub ocr_priority: OcrPriorityMode,
    /// Seconds before idle ONNX session is unloaded from VRAM/RAM (default: 180s).
    pub ocr_session_idle_unload_secs: u64,
    /// Optional custom ONNX model paths.
    pub custom_onnx_model: CustomOnnxModelConfig,
    /// Default GIF recording FPS (default: 15).
    pub default_gif_fps: u32,
}

impl Default for ScreenshotConfig {
    fn default() -> Self {
        Self {
            enable_manual_box_selection: true,
            enable_smart_region_detection: true,
            smart_region_wheel_expand: true,
            show_magnifier: true,
            ocr_priority: OcrPriorityMode::SystemNativeFirst,
            ocr_session_idle_unload_secs: 180,
            custom_onnx_model: CustomOnnxModelConfig {
                det_model_path: None,
                rec_model_path: None,
                keys_dict_path: None,
                hardware_fallback_order: vec![
                    "gpu".to_string(),
                    "npu".to_string(),
                    "cpu".to_string(),
                ],
            },
            default_gif_fps: 15,
        }
    }
}

/// Configuration for Module 02: Clipboard Manager (Ditto-style LRU-100).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClipboardConfig {
    pub enabled: bool,
    /// Maximum unpinned clipboard items retained in SQLite (default: 100).
    pub max_history_items: usize,
    pub capture_images: bool,
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_history_items: 100,
            capture_images: true,
        }
    }
}

/// Configuration for Module 05: App Usage Time Tracker (TAI-style).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UsageTrackingConfig {
    pub enabled: bool,
    /// Seconds of keyboard/mouse inactivity before pausing active timer (default: 300s).
    pub idle_threshold_secs: u64,
    /// Batch database flush interval in seconds (default: 60s).
    pub flush_interval_secs: u64,
}

impl Default for UsageTrackingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            idle_threshold_secs: 300,
            flush_interval_secs: 60,
        }
    }
}

/// Configuration for Module 10: Biometric Password Vault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PasswordVaultConfig {
    pub biometric_enabled: bool,
    /// Seconds of inactivity before zeroizing DEK in memory and locking the vault (default: 180s).
    pub auto_lock_secs: u64,
    /// Seconds before clearing copied password from system clipboard (default: 30s).
    pub clear_clipboard_after_secs: u64,
}

impl Default for PasswordVaultConfig {
    fn default() -> Self {
        Self {
            biometric_enabled: true,
            auto_lock_secs: 180,
            clear_clipboard_after_secs: 30,
        }
    }
}

/// Explicit user-configured binary paths overriding system `PATH` probing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomToolPathsConfig {
    pub ffmpeg_path: Option<PathBuf>,
    pub ffprobe_path: Option<PathBuf>,
    pub node_path: Option<PathBuf>,
    pub python3_path: Option<PathBuf>,
}

/// General daemon and UI preferences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    pub launch_at_login: bool,
    pub show_tray_icon: bool,
    pub language: String,
    pub theme: String,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            launch_at_login: false,
            show_tray_icon: true,
            language: "zh-CN".to_string(),
            theme: "system".to_string(),
        }
    }
}

/// Persisted runtime switches, independent of each feature's own settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModulesConfig {
    pub enabled: HashMap<String, bool>,
    /// Each feature owns the typed schema of its settings; core only persists JSON.
    pub settings: HashMap<String, Value>,
    #[serde(flatten)]
    pub unknown_fields: Map<String, Value>,
}
impl Default for ModulesConfig {
    fn default() -> Self {
        Self {
            enabled: crate::catalog::MODULE_CATALOG
                .iter()
                .map(|module| (module.id.to_string(), true))
                .collect(),
            settings: HashMap::new(),
            unknown_fields: Map::new(),
        }
    }
}
impl ModulesConfig {
    /// Custom modules can be mounted without adding a core configuration field.
    pub fn is_enabled(&self, id: &str) -> bool {
        self.enabled.get(id).copied().unwrap_or(true)
    }
}

/// Root versioned configuration structure persisted to `config.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub config_version: u32,
    pub general: GeneralConfig,
    pub modules: ModulesConfig,
    /// Map of `action_id` -> user custom logical hotkey override (e.g. `"screenshot:capture": "PrimaryMod+Alt+A"`).
    pub shortcut_overrides: HashMap<String, String>,
    pub custom_tool_paths: CustomToolPathsConfig,
    pub screenshot: ScreenshotConfig,
    pub clipboard: ClipboardConfig,
    pub usage_tracking: UsageTrackingConfig,
    pub password_vault: PasswordVaultConfig,
    /// Preserves unknown fields from newer/custom versions so saving never strips unrecognized keys.
    #[serde(flatten)]
    pub unknown_fields: Map<String, Value>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            config_version: CURRENT_CONFIG_VERSION,
            general: GeneralConfig::default(),
            modules: ModulesConfig::default(),
            shortcut_overrides: HashMap::new(),
            custom_tool_paths: CustomToolPathsConfig::default(),
            screenshot: ScreenshotConfig::default(),
            clipboard: ClipboardConfig::default(),
            usage_tracking: UsageTrackingConfig::default(),
            password_vault: PasswordVaultConfig::default(),
            unknown_fields: Map::new(),
        }
    }
}
