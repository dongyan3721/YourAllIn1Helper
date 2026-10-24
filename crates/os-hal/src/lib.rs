//! `os-hal` — Cross-platform OS Hardware Abstraction Layer (Bridge Pattern).
//!
//! Isolates all platform-specific APIs (`windows-rs` on Windows, `objc2` / CoreGraphics /
//! Vision / AppKit / LocalAuthentication on macOS) behind 7 zero-cost or injectable traits:
//! - [`ScreenCapturer`]: Display topology enumeration and raw RGBA frame capture.
//! - [`RegionDetector`]: Snipaste-style L1 (Z-Order window tree) & L2 (UIAutomation / AXUIElement) bounding-box detection.
//! - [`OcrProvider`]: System native OCR (`Windows.Media.Ocr` / macOS `Vision`) + ONNX hardware fallback chain.
//! - [`WindowController`]: External window handle picking & topmost pinning (`HWND_TOPMOST` / `CGWindowLevel`).
//! - [`SelectionExtractor`]: Dual-channel global selected text extraction (Accessibility first, simulated `Ctrl/Cmd+C` fallback).
//! - [`BiometricAuthenticator`]: Windows Hello (`UserConsentVerifier`) & macOS Touch ID (`LAContext`) verification.
//! - [`ActiveAppWatcher`]: Foreground active application tracking and idle/screen-lock detection.
//! - [`InputSimulator`]: Synthetic keyboard/mouse input (`Ctrl+V` / `Cmd+V` quick paste, auto-scroll wheel).

pub mod mock;
pub mod platform;
pub mod traits;
pub mod types;

pub use mock::MockOsHal;
pub use platform::NativeOsHal;
pub use traits::{
    ActiveAppWatcher, BiometricAuthenticator, InputSimulator, OcrProvider, OsHalBundle,
    RegionDetector, ScreenCapturer, SelectionExtractor, WindowController,
};
pub use types::{
    ActiveAppSnapshot, BiometricResult, CapturedFrame, DisplayMonitorInfo, ExternalWindowInfo,
    HalError, HalResult, OcrBackendKind, OcrLineResult, OcrRecognitionResult, SelectionSource,
    SelectionTextResult, WindowHandleId,
};

#[cfg(test)]
#[path = "../tests/unit/platform.rs"]
mod tests;
