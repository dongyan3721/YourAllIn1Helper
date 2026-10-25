//! 尚未实现的原生能力明确返回 Unsupported，禁止回退到测试数据。
use super::NativeOsHal;
use crate::{
    BiometricAuthenticator, BiometricResult, CapturedFrame, HalError, HalResult, OcrProvider,
    OcrRecognitionResult,
};

impl OcrProvider for NativeOsHal {
    fn recognize_rgba(&self, _frame: &CapturedFrame) -> HalResult<OcrRecognitionResult> {
        Err(HalError::Unsupported("native_ocr"))
    }

    fn release_idle_sessions(&self) -> HalResult<()> {
        Ok(())
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
