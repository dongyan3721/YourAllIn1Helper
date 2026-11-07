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

    for code in [
        NTE_BAD_FLAGS,
        NTE_NOT_SUPPORTED,
        NTE_PERM,
        0x1111_2222_u32 as i32,
    ] {
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
    assert_eq!(
        store.backend_byte(),
        crate::keystore::WINDOWS_HELLO_BACKEND_ID
    );
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
fn passport_key_name_is_scoped_to_user_application_and_slot() {
    use crate::windows_hello::format_key_name;
    let a = uuid::Uuid::from_bytes([1; 16]);
    let b = uuid::Uuid::from_bytes([2; 16]);
    let name = format_key_name("S-1-5-21-100-200-300-1001", &a);
    assert_eq!(
        name,
        format!("S-1-5-21-100-200-300-1001//com.allinone.desktop/vault/{a}")
    );
    assert_ne!(name, format_key_name("S-1-5-21-100-200-300-1002", &a));
    assert_ne!(name, format_key_name("S-1-5-21-100-200-300-1001", &b));
}

#[test]
fn invalid_owner_window_is_rejected_without_creating_a_window() {
    assert!(crate::windows_hello::set_owner_window(0).is_err());
    assert!(crate::windows_hello::set_owner_window(1).is_err());
}

#[test]
fn wrapped_key_padding_is_explicit_and_malformed_payloads_are_rejected() {
    use crate::windows_hello::{split_wrapped_key, RsaPadding};
    let mut old = vec![0x25; 256];
    old[..4].copy_from_slice(b"PKC1");
    assert_eq!(split_wrapped_key(&old).unwrap().0, RsaPadding::Oaep);
    let new = [b"PKC1".as_slice(), &[0x25; 256]].concat();
    assert_eq!(split_wrapped_key(&new).unwrap().0, RsaPadding::Pkcs1);
    for malformed in [&new[..259], &[0u8; 260][..], &[0u8; 32][..]] {
        assert!(split_wrapped_key(malformed).is_err());
    }
}

#[test]
#[ignore = "真实 Hello 往返：创建本应用虚构槽并弹出系统认证，由用户亲自完成"]
fn hardware_windows_hello_roundtrip() {
    let _log = tracing::subscriber::set_default(HardwareDiagnosticLog);
    use crate::hello_store::WindowsHelloKeyStore;
    use crate::keystore::VaultKeyStore;
    use crate::windows_hello::RealHello;
    let root = tempfile::tempdir().expect("isolated envelope directory");
    let store =
        WindowsHelloKeyStore::new(std::sync::Arc::new(RealHello), root.path().to_path_buf());
    let slot = uuid::Uuid::new_v4();
    // 固定虚构内容，绝不接触真实密码本。即使断言失败也尝试撤销本次独立槽。
    struct Cleanup<'a>(&'a WindowsHelloKeyStore, uuid::Uuid);
    impl Drop for Cleanup<'_> {
        fn drop(&mut self) {
            let _ = self.0.revoke(&self.1);
        }
    }
    let _cleanup = Cleanup(&store, slot);
    eprintln!("Hello QA: enrolling a new isolated device key");
    store
        .protect(&slot, &[0xA7; 32])
        .expect("enroll Hello device key");
    eprintln!("Hello QA: decrypting via Windows Hello; complete the system prompt yourself");
    assert_eq!(
        store.unprotect(&slot).expect("Hello unlock").as_slice(),
        &[0xA7; 32]
    );
    eprintln!("Hello QA: checking fresh gesture for a second unlock");
    assert_eq!(
        store
            .unprotect(&slot)
            .expect("second Hello unlock")
            .as_slice(),
        &[0xA7; 32]
    );
    store.revoke(&slot).expect("revoke test key");
    assert!(store.unprotect(&slot).is_err());
}

// 手工硬件测试输出原始错误码及操作名，不记录密码、密钥或密文。
struct HardwareDiagnosticLog;
impl tracing::Subscriber for HardwareDiagnosticLog {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        struct Fields;
        impl tracing::field::Visit for Fields {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                eprint!(" {}={value:?}", field.name());
            }
        }
        eprint!("Hello API:");
        event.record(&mut Fields);
        eprintln!();
    }
}

#[test]
#[ignore = "仅创建未 finalized 的随机测试槽，核对 Passport 密钥名，不发起认证"]
fn native_passport_accepts_scoped_key_name_without_authentication() {
    crate::windows_hello::probe_scoped_key_creation()
        .expect("Passport accepts SID-scoped key name");
}

#[test]
#[ignore = "创建并撤销虚构系统测试钥匙；静默诊断公钥填充能力，不调用交互解密"]
fn native_passport_padding_capabilities() {
    let _log = tracing::subscriber::set_default(HardwareDiagnosticLog);
    crate::windows_hello::diagnose_native_padding().expect("native padding diagnostic");
}
