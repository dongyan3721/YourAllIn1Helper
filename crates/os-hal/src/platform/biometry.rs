//! Touch ID 能力探测。只调用 `canEvaluatePolicy`，不弹窗，也不读取任何钥匙串秘密。
//! 认证成功与否不能单独解开密码本；解钥必须走带生物识别访问控制的钥匙串项。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchIdProbe {
    Ready,
    NotEnrolled,
    Lockout,
    Unavailable,
}

#[cfg(target_os = "macos")]
pub fn touch_id_probe() -> TouchIdProbe {
    #[link(name = "LocalAuthentication", kind = "framework")]
    extern "C" {}

    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool};

    let Some(class) = AnyClass::get(c"LAContext") else {
        return TouchIdProbe::Unavailable;
    };
    // deviceOwnerAuthenticationWithBiometrics：只承认 Touch ID / Face ID，不把开机密码当成等价项。
    const BIOMETRICS: isize = 1;
    let context: Retained<AnyObject> = unsafe { msg_send![class, new] };
    let mut error: *mut AnyObject = std::ptr::null_mut();
    let ok: Bool =
        unsafe { msg_send![&*context, canEvaluatePolicy: BIOMETRICS, error: &mut error] };
    if ok.as_bool() {
        return TouchIdProbe::Ready;
    }
    if error.is_null() {
        return TouchIdProbe::Unavailable;
    }
    let code: isize = unsafe { msg_send![error, code] };
    // SAFETY: canEvaluatePolicy 失败时把 NSError 所有权交给我们，这里只为读取后释放。
    let _error = unsafe { Retained::from_raw(error) };
    match code {
        -7 => TouchIdProbe::NotEnrolled,
        -8 => TouchIdProbe::Lockout,
        _ => TouchIdProbe::Unavailable,
    }
}

#[cfg(not(target_os = "macos"))]
pub fn touch_id_probe() -> TouchIdProbe {
    TouchIdProbe::Unavailable
}
