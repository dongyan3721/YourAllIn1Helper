//! 显式本地 HTTP Fixture 协议与流式测试。
//!
//! 规范要求：
//! 1. 不读取用户真实 API Key，不擅自向外部网络发送选中文本；
//! 2. 严禁声称外部 API 实测通过；使用显式本地 HTTP 服务模拟 Google/DeepL/OpenAI 官方 HTTP 协议、头规范、SSE 流式分片与跨块 UTF-8。

use mod_translator::{
    credentials::{CredentialStorage, SessionCredentialStore},
    provider::{
        deepl::DeepLProvider, google::GoogleProvider, openai::OpenAIProvider, TranslationProvider,
        TranslationRequest, TranslationStreamEvent,
    },
    service::TranslatorService,
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

#[tokio::test]
async fn test_google_protocol_fixture() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut buf = vec![0u8; 2048];
        let n = socket.read(&mut buf).unwrap();
        let request_str = String::from_utf8_lossy(&buf[..n]);

        // 校验请求头与方法
        assert!(request_str.starts_with("POST /v2/translate"));
        assert!(request_str.contains("x-goog-api-key: mock-google-key"));
        assert!(request_str.contains(r#""q":["All in One"]"#));
        assert!(request_str.contains(r#""target":"zh""#));

        let body = r#"{"data":{"translations":[{"translatedText":"多合一工具箱"}]}}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        socket.write_all(response.as_bytes()).unwrap();
    });

    let endpoint = format!("http://{}/v2/translate", addr);
    let provider = GoogleProvider::with_endpoint(endpoint);
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let req = TranslationRequest {
        text: "All in One".to_string(),
        from: "en".to_string(),
        to: "zh-CN".to_string(),
        stream: false,
    };

    let result = provider
        .translate(&client, &req, Some("mock-google-key"))
        .await
        .unwrap();
    assert_eq!(result, "多合一工具箱");

    server.join().unwrap();
}

#[tokio::test]
async fn test_deepl_protocol_fixture() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut buf = vec![0u8; 2048];
        let n = socket.read(&mut buf).unwrap();
        let request_str = String::from_utf8_lossy(&buf[..n]);

        assert!(request_str.starts_with("POST /v2/translate"));
        assert!(request_str.contains("authorization: DeepL-Auth-Key mock-deepl-key:fx"));
        assert!(request_str.contains(r#""text":["Good afternoon"]"#));
        assert!(request_str.contains(r#""target_lang":"ZH""#));

        let body = r#"{"translations":[{"text":"下午好"}]}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        socket.write_all(response.as_bytes()).unwrap();
    });

    let endpoint = format!("http://{}/v2/translate", addr);
    let provider = DeepLProvider::with_endpoint(endpoint);
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let req = TranslationRequest {
        text: "Good afternoon".to_string(),
        from: "en".to_string(),
        to: "zh-CN".to_string(),
        stream: false,
    };

    let result = provider
        .translate(&client, &req, Some("mock-deepl-key:fx"))
        .await
        .unwrap();
    assert_eq!(result, "下午好");

    server.join().unwrap();
}

#[tokio::test]
async fn test_openai_sse_streaming_protocol_fixture() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut buf = vec![0u8; 2048];
        let n = socket.read(&mut buf).unwrap();
        let request_str = String::from_utf8_lossy(&buf[..n]);

        assert!(request_str.starts_with("POST /v1/chat/completions"));
        assert!(request_str.contains("authorization: Bearer mock-openai-key"));
        assert!(request_str.contains(r#""stream":true"#));

        // 响应 HTTP 200 SSE 流式头
        let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
        socket.write_all(header.as_bytes()).unwrap();

        // 辅助闭包：发送 chunked 块
        let mut send_chunk = |data: &str| {
            let len_hex = format!("{:X}\r\n", data.len());
            socket.write_all(len_hex.as_bytes()).unwrap();
            socket.write_all(data.as_bytes()).unwrap();
            socket.write_all(b"\r\n").unwrap();
        };

        // 发送分片 1
        send_chunk("data: {\"choices\":[{\"delta\":{\"content\":\"人工\"}}]}\n\n");
        thread::sleep(Duration::from_millis(20));

        // 发送分片 2
        send_chunk("data: {\"choices\":[{\"delta\":{\"content\":\"智能\"}}]}\n\n");
        thread::sleep(Duration::from_millis(20));

        // 发送完成标记
        send_chunk("data: [DONE]\n\n");
        socket.write_all(b"0\r\n\r\n").unwrap();
    });

    let endpoint = format!("http://{}/v1/chat/completions", addr);
    let provider = OpenAIProvider::custom(
        "openai",
        "OpenAI",
        endpoint,
        Some("gpt-4o".to_string()),
        Some(0.2),
    );

    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let req = TranslationRequest {
        text: "Artificial Intelligence".to_string(),
        from: "en".to_string(),
        to: "zh-CN".to_string(),
        stream: true,
    };

    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    let handle = tokio::spawn(async move {
        provider
            .translate_stream(&client, &req, Some("mock-openai-key"), tx)
            .await
            .unwrap();
    });

    let mut chunks = Vec::new();
    let mut done_text = String::new();

    while let Some(ev) = rx.recv().await {
        match ev {
            TranslationStreamEvent::Chunk(c) => chunks.push(c),
            TranslationStreamEvent::Done(full) => done_text = full,
            TranslationStreamEvent::Error(e) => panic!("不应收到错误: {}", e),
        }
    }

    handle.await.unwrap();
    server.join().unwrap();

    assert_eq!(chunks, vec!["人工".to_string(), "智能".to_string()]);
    assert_eq!(done_text, "人工智能");
}

#[tokio::test]
async fn test_cancellation_and_discard_late_fixture() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    // 服务器故意延迟 500ms 响应
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut buf = vec![0u8; 1024];
        let _ = socket.read(&mut buf);
        thread::sleep(Duration::from_millis(300));

        let body = r#"{"data":{"translations":[{"translatedText":"迟到的结果"}]}}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = socket.write_all(response.as_bytes());
    });

    let creds = Arc::new(SessionCredentialStore::new());
    creds.set_api_key("google", "test-key").unwrap();
    let service = Arc::new(TranslatorService::new(creds));

    let endpoint = format!("http://{}/v2/translate", addr);
    service.register_provider(Arc::new(GoogleProvider::with_endpoint(endpoint)));

    let received_events = Arc::new(AtomicUsize::new(0));
    let counter = received_events.clone();
    let sink = Arc::new(move |_id: u64, _ev: TranslationStreamEvent| {
        counter.fetch_add(1, Ordering::SeqCst);
    });

    let req = TranslationRequest {
        text: "Late text".to_string(),
        from: "en".to_string(),
        to: "zh-CN".to_string(),
        stream: false,
    };

    // 发起请求
    let _ = service
        .execute_translation("google", req, sink.clone())
        .await
        .unwrap();

    // 等待 30ms 确保 HTTP 请求已发送并被本地服务端接收
    tokio::time::sleep(Duration::from_millis(30)).await;

    // 立即取消
    service.cancel_current();

    // 等待服务端延迟超时
    tokio::time::sleep(Duration::from_millis(400)).await;

    // 校验：迟到响应被丢弃，sink 收到事件数为 0
    assert_eq!(received_events.load(Ordering::SeqCst), 0);

    let _ = server.join();
}

// [DONE] 即为结束，不依赖服务端关闭持久连接。
#[tokio::test]
async fn test_sse_done_completes_before_connection_closes() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0; 4096];
        socket.read(&mut request).unwrap();
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").unwrap();
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n";
        write!(socket, "{:X}\r\n{}\r\n", body.len(), body).unwrap();
        thread::sleep(Duration::from_millis(500));
        let _ = socket.write_all(b"0\r\n\r\n");
    });
    let provider = OpenAIProvider::custom(
        "owned",
        "Owned fixture",
        format!("http://{addr}/v1/chat/completions"),
        None,
        None,
    );
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let req = TranslationRequest {
        text: "test".into(),
        from: "en".into(),
        to: "zh".into(),
        stream: true,
    };
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    tokio::time::timeout(
        Duration::from_millis(300),
        provider.translate_stream(&client, &req, None, tx),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        rx.recv().await,
        Some(TranslationStreamEvent::Chunk("ok".into()))
    );
    assert_eq!(
        rx.recv().await,
        Some(TranslationStreamEvent::Done("ok".into()))
    );
    server.join().unwrap();
}

// 验证正式 Client 确实发出 TLS ClientHello；不读取信任库秘密，也不访问外网。
#[tokio::test]
async fn test_https_transport_attempts_tls_handshake() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            match listener.accept() {
                Ok((mut socket, _)) => {
                    socket.set_nonblocking(false).unwrap();
                    socket
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    let mut record = [0; 3];
                    socket.read_exact(&mut record).unwrap();
                    assert_eq!(record[0], 22, "TLS handshake record");
                    assert_eq!(record[1], 3, "TLS legacy record version");
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "HTTPS connector never attempted TLS"
                    );
                    thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("owned listener failed: {e}"),
            }
        }
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    // 服务端主动结束握手，因此这里预期错误；上面的字节断言证明启用了 TLS。
    assert!(client.get(format!("https://{addr}/")).send().await.is_err());
    server.join().unwrap();
}

#[tokio::test]
async fn test_sse_service_error_and_truncated_stream_are_not_success() {
    for body in [
        "data: {\"error\":{\"message\":\"fixture rejected\"}}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n",
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            socket.read(&mut request).unwrap();
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        });
        let provider = OpenAIProvider::custom(
            "owned",
            "Owned",
            format!("http://{addr}/v1/chat/completions"),
            None,
            None,
        );
        let req = TranslationRequest {
            text: "test".into(),
            from: "en".into(),
            to: "zh".into(),
            stream: true,
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        assert!(provider
            .translate_stream(&client, &req, None, tx)
            .await
            .is_err());
        let mut saw_error = false;
        while let Some(event) = rx.recv().await {
            assert!(!matches!(event, TranslationStreamEvent::Done(_)));
            saw_error |= matches!(event, TranslationStreamEvent::Error(_));
        }
        assert!(saw_error);
        server.join().unwrap();
    }
}
