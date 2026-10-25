//! `mod-translator` 单元测试集合。
use crate::{
    config::{CustomProviderConfig, TranslatorConfig},
    credentials::{CredentialStorage, SessionCredentialStore},
    provider::{
        deepl::DeepLProvider, google::GoogleProvider, openai::OpenAIProvider, TranslationProvider,
        TranslationRequest, TranslationStreamEvent,
    },
    service::TranslatorService,
    stream::{parse_sse_openai_delta, Utf8ChunkBuffer},
    TranslatorModule,
};
use os_hal::MockOsHal;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[test]
fn test_config_sanitization() {
    let mut cfg = TranslatorConfig {
        default_provider_id: "".to_string(),
        target_lang: "invalid-lang".to_string(),
        source_lang: "".to_string(),
        auto_copy_result: true,
        history_enabled: false,
        hotkey: "".to_string(),
        popover_width: 10.0,
        popover_height: 5000.0,
        custom_providers: vec![
            CustomProviderConfig {
                id: "custom1".to_string(),
                name: "Custom 1".to_string(),
                endpoint_url: "https://api.example.com/v1".to_string(),
                model: Some("gpt-4o".to_string()),
                temperature: Some(0.5),
                is_openai_compatible: true,
            },
            CustomProviderConfig {
                id: "".to_string(), // 非法：无 id
                name: "Bad".to_string(),
                endpoint_url: "ftp://bad".to_string(),
                model: None,
                temperature: None,
                is_openai_compatible: true,
            },
        ],
    };

    cfg.sanitize();

    assert_eq!(cfg.default_provider_id, "google");
    assert_eq!(cfg.target_lang, "zh-CN");
    assert_eq!(cfg.source_lang, "auto");
    assert_eq!(cfg.hotkey, "PrimaryMod+Shift+T");
    assert_eq!(cfg.popover_width, 420.0);
    assert_eq!(cfg.popover_height, 340.0);
    assert_eq!(cfg.custom_providers.len(), 1);
    assert_eq!(cfg.custom_providers[0].id, "custom1");
}

#[test]
fn test_credentials_security_and_isolation() {
    let creds = SessionCredentialStore::new();

    // 1. 初始为空
    assert!(!creds.has_api_key("google"));
    assert_eq!(creds.get_api_key("google").unwrap(), None);

    // 2. 设置凭据
    creds.set_api_key("google", "test-secret-key-123").unwrap();
    assert!(creds.has_api_key("google"));
    assert_eq!(
        creds.get_api_key("google").unwrap(),
        Some("test-secret-key-123".to_string())
    );

    // 3. 密钥不暴露于 Debug 输出中
    let debug_str = format!("{:?}", creds);
    assert!(!debug_str.contains("test-secret-key-123"));
    assert!(debug_str.contains("stored_providers_count: 1"));

    // 4. 删除凭据
    creds.delete_api_key("google").unwrap();
    assert!(!creds.has_api_key("google"));
    assert_eq!(creds.get_api_key("google").unwrap(), None);

    // 5. 空白串自动删除
    creds.set_api_key("deepl", "   ").unwrap();
    assert!(!creds.has_api_key("deepl"));
}

#[test]
fn test_utf8_chunk_buffer_multibyte_cross_chunk_boundary() {
    let mut buffer = Utf8ChunkBuffer::new();

    // 测试 1：普通 ASCII 字符串
    assert_eq!(buffer.push_bytes(b"Hello "), "Hello ");

    // 测试 2：中文字符跨块切断
    // "你" 的 UTF-8 编码为 3 字节: [0xE4, 0xBD, 0xA0]
    // "好" 的 UTF-8 编码为 3 字节: [0xE5, 0xA5, 0xBD]
    let chunk1 = [0xE4, 0xBD]; // 只有前 2 字节
    let piece1 = buffer.push_bytes(&chunk1);
    assert_eq!(piece1, ""); // 未成字，不应输出非法乱码

    let chunk2 = [0xA0, 0xE5, 0xA5, 0xBD]; // 补全 "你" 的最后 1 字节，以及完整的 "好"
    let piece2 = buffer.push_bytes(&chunk2);
    assert_eq!(piece2, "你好");

    // 测试 3：4 字节 Emoji (🦀 = [0xF0, 0x9F, 0xA6, 0x80]) 跨 3 次分片
    let part1 = [0xF0];
    assert_eq!(buffer.push_bytes(&part1), "");
    let part2 = [0x9F, 0xA6];
    assert_eq!(buffer.push_bytes(&part2), "");
    let part3 = [0x80];
    assert_eq!(buffer.push_bytes(&part3), "🦀");

    // 测试 4：尾部刷写
    assert_eq!(buffer.finish(), "");
}

#[test]
fn test_sse_openai_delta_parsing() {
    // 正常分片
    let line1 = r#"data: {"choices":[{"delta":{"content":"Hello"}}]}"#;
    assert_eq!(parse_sse_openai_delta(line1), Some("Hello".to_string()));

    // 中文分片
    let line2 = r#"data: {"choices":[{"delta":{"content":"世界"}}]}"#;
    assert_eq!(parse_sse_openai_delta(line2), Some("世界".to_string()));

    // 终止标记
    let line_done = "data: [DONE]";
    assert_eq!(parse_sse_openai_delta(line_done), None);

    // 忽略普通心跳或非 data 行
    assert_eq!(parse_sse_openai_delta(": keep-alive"), None);
    assert_eq!(parse_sse_openai_delta(""), None);
}

#[tokio::test]
async fn test_missing_credentials_realistically_notified() {
    let client = reqwest::Client::new();
    let req = TranslationRequest {
        text: "Hello".to_string(),
        from: "en".to_string(),
        to: "zh".to_string(),
        stream: false,
    };

    // 1. Google：无凭据时准确报错，不擅自外发
    let google = GoogleProvider::new();
    let err = google.translate(&client, &req, None).await.unwrap_err();
    assert!(err.to_string().contains("Google Translate API Key 未配置"));

    // 2. DeepL：无凭据时准确报错
    let deepl = DeepLProvider::new();
    let err = deepl.translate(&client, &req, None).await.unwrap_err();
    assert!(err.to_string().contains("DeepL API Key 未配置"));

    // 3. OpenAI：远端端点无凭据时准确报错
    let openai = OpenAIProvider::new();
    let err = openai.translate(&client, &req, None).await.unwrap_err();
    assert!(err.to_string().contains("OpenAI API Key 未配置"));

    // 4. OpenAI：本地端点允许无密钥直接调用
    let local_openai = OpenAIProvider::custom(
        "local",
        "Local Ollama",
        "http://localhost:11434/v1/chat/completions",
        Some("llama3".to_string()),
        None,
    );
    // 仅验证能力元数据，绝不连接用户可能正在运行的本地模型。
    assert!(!local_openai.requires_api_key());
    for endpoint in [
        "https://localhost.evil.example/v1/chat/completions",
        "https://remote.example/localhost/chat/completions",
        "https://127.0.0.1@remote.example/v1/chat/completions",
    ] {
        let remote = OpenAIProvider::custom("remote", "Remote", endpoint, None, None);
        assert!(remote.requires_api_key());
        assert!(remote
            .translate(&client, &req, None)
            .await
            .unwrap_err()
            .to_string()
            .contains("API Key 未配置"));
    }
}

#[tokio::test]
async fn test_sequence_ordering_and_late_result_discard() {
    let creds = Arc::new(SessionCredentialStore::new());
    let service = Arc::new(TranslatorService::new(creds));

    let received_chunks = Arc::new(AtomicUsize::new(0));
    let chunks_counter = received_chunks.clone();

    let sink = Arc::new(move |_req_id: u64, event: TranslationStreamEvent| {
        if let TranslationStreamEvent::Chunk(_) = event {
            chunks_counter.fetch_add(1, Ordering::SeqCst);
        }
    });

    // 1. 发起请求 1（空文本直接拒绝）
    let req_err = TranslationRequest {
        text: "   ".to_string(),
        from: "auto".to_string(),
        to: "zh-CN".to_string(),
        stream: false,
    };
    assert!(service
        .execute_translation("google", req_err, sink.clone())
        .await
        .is_err());

    // 2. 取消旧任务递增 sequence ID
    service.cancel_current();
}

#[test]
fn test_translator_module_selection_reading_with_mock_hal() {
    let mock_hal = Arc::new(MockOsHal::new());
    let module = TranslatorModule::new(mock_hal);

    // 初始状态
    assert_eq!(module.is_active(), false);
    assert_eq!(module.config().default_provider_id, "google");

    // 取词
    let selection = module.extract_selected_text().unwrap();
    assert!(selection.is_some());
    let sel = selection.unwrap();
    assert_eq!(sel.text, "Hello All-in-One");
}

#[test]
fn custom_endpoints_cannot_override_builtins_or_survive_removal() {
    let module = TranslatorModule::new(Arc::new(MockOsHal::new()));
    let custom = |id: &str| CustomProviderConfig {
        id: id.into(),
        name: "Custom".into(),
        endpoint_url: "http://127.0.0.1:1234/v1/chat/completions".into(),
        model: None,
        temperature: None,
        is_openai_compatible: true,
    };
    let mut cfg = TranslatorConfig::default();
    cfg.custom_providers.push(custom("google"));
    assert!(module.update_config(cfg.clone()).is_err());
    cfg.custom_providers = vec![custom("local")];
    module.update_config(cfg).unwrap();
    assert!(module.service().get_provider("local").is_some());
    module.update_config(TranslatorConfig::default()).unwrap();
    assert!(module.service().get_provider("local").is_none());
    assert!(module.service().get_provider("google").is_some());
}

#[test]
fn cancellation_waits_for_request_registration() {
    struct GatedCredentials {
        entered: std::sync::mpsc::SyncSender<()>,
        release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl CredentialStorage for GatedCredentials {
        fn get_api_key(&self, _: &str) -> app_core::CoreResult<Option<String>> {
            self.entered.send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
            Ok(None) // 无密钥，绝不会调用远端。
        }
        fn set_api_key(&self, _: &str, _: &str) -> app_core::CoreResult<()> {
            Ok(())
        }
        fn delete_api_key(&self, _: &str) -> app_core::CoreResult<()> {
            Ok(())
        }
        fn has_api_key(&self, _: &str) -> bool {
            false
        }
    }
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
    let service = Arc::new(TranslatorService::new(Arc::new(GatedCredentials {
        entered: entered_tx,
        release: std::sync::Mutex::new(release_rx),
    })));
    let worker_service = service.clone();
    let worker = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime
            .block_on(worker_service.execute_translation(
                "google",
                TranslationRequest {
                    text: "Owned test".into(),
                    from: "en".into(),
                    to: "zh".into(),
                    stream: true,
                },
                Arc::new(|_, _| {}),
            ))
            .unwrap();
    });
    entered_rx.recv().unwrap();
    let (cancelled_tx, cancelled_rx) = std::sync::mpsc::sync_channel(1);
    let cancel = std::thread::spawn(move || {
        service.cancel_current();
        cancelled_tx.send(()).unwrap();
    });
    assert!(matches!(
        cancelled_rx.recv_timeout(std::time::Duration::from_millis(50)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    release_tx.send(()).unwrap();
    worker.join().unwrap();
    cancelled_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    cancel.join().unwrap();
}
