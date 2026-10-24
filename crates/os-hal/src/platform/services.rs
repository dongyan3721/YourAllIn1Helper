//! 尚未实现的原生能力明确返回 Unsupported，禁止回退到测试数据。
use super::NativeOsHal;
use crate::{
    ActiveAppSnapshot, ActiveAppWatcher, BiometricAuthenticator, BiometricResult, CapturedFrame,
    HalError, HalResult, OcrProvider, OcrRecognitionResult, SelectionExtractor,
    SelectionTextResult, WindowController,
};

impl OcrProvider for NativeOsHal {
    fn recognize_rgba(&self, _frame: &CapturedFrame) -> HalResult<OcrRecognitionResult> {
        Err(HalError::Unsupported("native_ocr"))
    }

    fn release_idle_sessions(&self) -> HalResult<()> {
        Ok(())
    }
}

impl SelectionExtractor for NativeOsHal {
    fn extract_selected_text(&self) -> HalResult<Option<SelectionTextResult>> {
        Err(HalError::Unsupported("native_selection_extraction"))
    }
}

impl BiometricAuthenticator for NativeOsHal {
    fn is_biometric_available(&self) -> bool {
        false
    }

    fn verify_identity(&self, _reason: &str) -> HalResult<BiometricResult> {
        Err(HalError::Unsupported("native_biometric_authentication"))
    }
}

impl ActiveAppWatcher for NativeOsHal {
    fn current_active_app(&self) -> HalResult<ActiveAppSnapshot> {
        let fg = self.get_foreground_window()?;
        Ok(ActiveAppSnapshot {
            app_id: format!("app.{}", fg.process_name.to_ascii_lowercase()),
            app_name: fg.process_name,
            window_title: fg.title,
            executable_path: None,
            process_id: fg.process_id,
        })
    }

    fn system_idle_seconds(&self) -> HalResult<u64> {
        Err(HalError::Unsupported("native_idle_detection"))
    }

    fn is_screen_locked(&self) -> HalResult<bool> {
        Err(HalError::Unsupported("native_screen_lock_detection"))
    }
}
