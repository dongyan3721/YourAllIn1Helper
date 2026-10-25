//! 尚未实现的原生能力明确返回 Unsupported，禁止回退到测试数据。
use super::NativeOsHal;
use crate::{BiometricAuthenticator, BiometricResult, HalError, HalResult};

impl BiometricAuthenticator for NativeOsHal {
    fn is_biometric_available(&self) -> bool {
        false
    }

    fn verify_identity(&self, _reason: &str) -> HalResult<BiometricResult> {
        Err(HalError::Unsupported("native_biometric_authentication"))
    }
}
