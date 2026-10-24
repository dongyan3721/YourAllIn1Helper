//! Domain types shared across `os-hal` traits and feature modules.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Unified result type for OS HAL operations.
pub type HalResult<T> = Result<T, HalError>;

/// Platform-neutral opaque identifier for an external OS window (`HWND` on Windows, `CGWindowID` on macOS).
pub type WindowHandleId = u64;

/// Errors originating from native OS HAL adapters.
#[derive(Debug, Error)]
pub enum HalError {
    #[error("OS permission denied for '{capability}': {guidance}")]
    PermissionDenied {
        capability: &'static str,
        guidance: String,
    },

    #[error("Requested OS resource not found: {0}")]
    NotFound(String),

    #[error("Platform capability '{0}' is not supported or unavailable on this system")]
    Unsupported(&'static str),

    #[error("Native OS API call failed: {0}")]
    NativeCallFailed(String),
}

/// Physical monitor topology metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayMonitorInfo {
    pub id: u32,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f64,
    pub is_primary: bool,
}

/// Frozen RGBA8 screen capture frame buffer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub scale_factor_milli: u32,
    /// Raw RGBA8 pixel bytes (`width * height * 4`).
    pub rgba_bytes: Vec<u8>,
}

/// Backend engine that fulfilled an OCR request in the fallback chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OcrBackendKind {
    /// Windows.Media.Ocr or macOS Vision framework (`VNRecognizeTextRequest`).
    SystemNative,
    /// ONNX Runtime GPU provider (`CUDA` / `DirectML` / `CoreML GPU`).
    OnnxGpu,
    /// ONNX Runtime NPU provider (`DirectML NPU` / `OpenVINO NPU` / `Apple Neural Engine`).
    OnnxNpu,
    /// ONNX Runtime CPU fallback (`AVX2` / `NEON`).
    OnnxCpu,
}

/// Single line recognized by OCR with bounding box coordinates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrLineResult {
    pub text: String,
    pub confidence: f32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Complete OCR recognition result returned to `mod-screenshot`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrRecognitionResult {
    pub backend_used: OcrBackendKind,
    pub full_text: String,
    pub lines: Vec<OcrLineResult>,
    pub elapsed_ms: u64,
}

/// Metadata describing an external third-party OS window (used by `mod-window-pin`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalWindowInfo {
    pub handle: WindowHandleId,
    pub title: String,
    pub process_name: String,
    pub process_id: u32,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub is_topmost: bool,
}

/// Channel that produced the extracted global text selection (used by `mod-translator`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionSource {
    /// Zero-clipboard-pollution native accessibility API (`UIAutomation` / `AXUIElement`).
    AccessibilityApi,
    /// Fallback simulated `Ctrl+C` / `Cmd+C` with clipboard backup & restore.
    SimulatedCopyFallback,
}

/// Selected text payload returned by `SelectionExtractor`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionTextResult {
    pub text: String,
    pub source: SelectionSource,
    pub source_app_name: Option<String>,
}

/// Outcome of a biometric / device-owner authentication prompt (`mod-password-vault`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BiometricResult {
    Verified,
    CancelledByUser,
    NotEnrolled,
    Failed(String),
}

/// Snapshot of the current foreground application (`mod-app-usage`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveAppSnapshot {
    /// Canonical identifier (Bundle ID on macOS, normalized executable path on Windows).
    pub app_id: String,
    pub app_name: String,
    pub window_title: String,
    pub executable_path: Option<String>,
    pub process_id: u32,
}
