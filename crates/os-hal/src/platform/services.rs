//! 尚未实现的原生能力明确返回 Unsupported，禁止回退到测试数据。
use super::NativeOsHal;
use crate::{BiometricAuthenticator, BiometricResult, HalError, HalResult};

impl BiometricAuthenticator for NativeOsHal {
    fn is_biometric_available(&self) -> bool {
        // 只报告当前生物识别能否使用。这个布尔值不能解开密码本。
        matches!(super::touch_id_probe(), super::TouchIdProbe::Ready)
    }

    fn verify_identity(&self, _reason: &str) -> HalResult<BiometricResult> {
        // 密码本解锁必须读取带 biometryCurrentSet 访问控制的钥匙串项。
        // 这里继续拒绝“认证成功就放行”的接口，避免调用方把它当成设备密钥。
        Err(HalError::Unsupported("native_biometric_authentication"))
    }
}
