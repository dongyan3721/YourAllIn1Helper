//! Windows Hello 分类与能力探测。不创建 NGC 密钥，不弹出认证框，不读取用户秘密。
use crate::error::VaultError;
use crate::windows_hello::{
    classify_silent_hresult, gesture_gate_holds, map_ncrypt_status, production_keystore,
    SilentDecrypt,
};

const NTE_BAD_FLAGS: i32 = 0x8009_0009_u32 as i32;
const NTE_NOT_SUPPORTED: i32 = 0x8009_0029_u32 as i32;
const NTE_SILENT_CONTEXT: i32 = 0x8009_0022_u32 as i32;
const NTE_PERM: i32 = 0x8009_0010_u32 as i32;

#[test]
fn silent_classification_keeps_raw_hresults_apart() {
    assert_eq!(
        classify_silent_hresult(Some(NTE_SILENT_CONTEXT)),
        SilentDecrypt::GestureRequired
    );
    assert!(gesture_gate_holds(classify_silent_hresult(Some(
        NTE_SILENT_CONTEXT
    ))));

    for code in [NTE_BAD_FLAGS, NTE_NOT_SUPPORTED, NTE_PERM, 0x1111_2222_u32 as i32] {
        let classified = classify_silent_hresult(Some(code));
        assert_eq!(classified, SilentDecrypt::Failed, "code {code:#x}");
        assert!(!gesture_gate_holds(classified));
    }
    assert_eq!(
        classify_silent_hresult(None),
        SilentDecrypt::PlaintextReturned
    );
    assert!(!gesture_gate_holds(classify_silent_hresult(None)));

    // 这两类都会变成“不可用”，但不能再从该错误反推成 NTE_SILENT_CONTEXT。
    assert!(matches!(
        map_ncrypt_status(NTE_BAD_FLAGS),
        VaultError::BiometricUnavailable
    ));
    assert!(matches!(
        map_ncrypt_status(NTE_NOT_SUPPORTED),
        VaultError::BiometricUnavailable
    ));
    assert!(matches!(
        map_ncrypt_status(NTE_SILENT_CONTEXT),
        VaultError::BiometricUnavailable
    ));
    assert_ne!(
        classify_silent_hresult(Some(NTE_BAD_FLAGS)),
        classify_silent_hresult(Some(NTE_SILENT_CONTEXT))
    );
    assert_ne!(
        classify_silent_hresult(Some(NTE_NOT_SUPPORTED)),
        classify_silent_hresult(Some(NTE_SILENT_CONTEXT))
    );
}

#[test]
fn windows_hello_source_does_not_release_dkek_from_dpapi_or_a_consent_bool() {
    let source = include_str!("../../src/windows_hello.rs");
    for forbidden in [
        "CryptProtectData",
        "CryptUnprotectData",
        "CredWrite",
        "CredRead",
        "RequestVerification",
        "NCryptProtectSecret",
        "NCryptUnprotectSecret",
        "ncrypt_code",
    ] {
        assert!(
            !source.contains(forbidden),
            "{forbidden} must not gate or store the device key"
        );
    }
    assert!(source.contains("classify_silent_hresult"));
    assert!(source.contains("NTE_SILENT_CONTEXT"));
}

#[test]
fn production_keystore_identifies_hello_without_creating_a_key() {
    let store = production_keystore();
    assert_eq!(store.backend_id(), crate::keystore::WINDOWS_HELLO_BACKEND);
    assert_eq!(store.backend_byte(), crate::keystore::WINDOWS_HELLO_BACKEND_ID);
    assert_eq!(
        store.device_key_kind(),
        crate::DeviceKeyKind::WindowsHelloGesture
    );
    // 只探测提供程序和 Hello 是否已配置。CheckAvailability 不弹认证框，也不创建密钥。
    match store.gate() {
        Ok(()) => eprintln!("windows hello gate: configured"),
        Err(error) => eprintln!("windows hello gate: {error}"),
    }
}

#[test]
#[ignore = "会创建 NGC 密钥并可能弹出 Windows Hello，禁止在普通测试中运行"]
fn hardware_windows_hello_roundtrip_is_not_run_automatically() {
    // 故意留空。真机验收必须由人在场执行，不能在自动化里完成手势。
}
