//! 翻译凭据存储切换与钥匙串适配测试（显式测试适配器，不访问真实钥匙串）。
use super::*;
use os_hal::MockSecureServices;

#[test]
fn defaults_to_session_and_switches_to_system_store() {
    let host = HostCredentialStore::new_session();
    assert_eq!(host.backend_kind(), CredentialBackend::Session);
    host.set_api_key("deepl", "session-only").unwrap();

    let secrets = Arc::new(MockSecureServices::new());
    assert!(host.use_system_store(secrets.clone(), TRANSLATOR_KEYCHAIN_SERVICE));
    assert_eq!(host.backend_kind(), CredentialBackend::SystemKeychain);
    // 会话内存中的旧值不迁移到钥匙串。
    assert!(!host.has_api_key("deepl"));

    host.set_api_key("deepl", "  fictional-key  ").unwrap();
    assert!(host.has_api_key("deepl"));
    assert_eq!(
        host.get_api_key("deepl").unwrap().as_deref(),
        Some("fictional-key")
    );
    assert_eq!(
        secrets
            .get_secret(TRANSLATOR_KEYCHAIN_SERVICE, "deepl")
            .unwrap()
            .as_deref(),
        Some(&b"fictional-key"[..])
    );
    host.set_api_key("deepl", "   ").unwrap();
    assert!(!host.has_api_key("deepl"));
    assert_eq!(
        secrets
            .get_secret(TRANSLATOR_KEYCHAIN_SERVICE, "deepl")
            .unwrap(),
        None
    );
}

#[test]
fn unavailable_system_store_keeps_session_backend() {
    let host = HostCredentialStore::new_session();
    assert!(!host.use_system_store(
        Arc::new(MockSecureServices::without_secret_store()),
        TRANSLATOR_KEYCHAIN_SERVICE,
    ));
    assert_eq!(host.backend_kind(), CredentialBackend::Session);
}
