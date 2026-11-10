//! Explicit, opt-in real-service QA. Credentials are never copied into fixtures.
use mod_translator::{
    credentials::{CredentialStorage, SessionCredentialStore},
    provider::{openai::OpenAIProvider, TranslationRequest, TranslationStreamEvent},
    service::TranslatorService,
};
use std::{collections::HashMap, sync::Arc, time::Duration};

#[tokio::test]
#[ignore = "Requires explicitly authorized ALL_IN_ONE_QA_LLM_ENV_FILE and real network access"]
async fn configured_custom_provider_translates_json_and_sse() {
    let path =
        std::env::var("ALL_IN_ONE_QA_LLM_ENV_FILE").expect("Set the authorized env-file path");
    let raw = std::fs::read_to_string(path).expect("Cannot read authorized env file");
    let vars: HashMap<_, _> = raw
        .lines()
        .filter_map(|line| {
            let (k, v) = line.trim().split_once('=')?;
            Some((k.trim(), v.trim().trim_matches('"').trim_matches('\'')))
        })
        .collect();
    let key = *vars.get("LLM_API_KEY").expect("Missing LLM_API_KEY");
    assert!(!key.is_empty(), "Empty API key");
    let endpoint = *vars.get("LLM_BASE_URL").expect("Missing LLM_BASE_URL");
    let model = *vars.get("LLM_MODEL").expect("Missing LLM_MODEL");
    let credentials = Arc::new(SessionCredentialStore::new());
    credentials.set_api_key("qa_live", key).unwrap();
    let service = TranslatorService::new(credentials.clone());
    service.register_provider(Arc::new(OpenAIProvider::custom(
        "qa_live",
        "Authorized live QA",
        endpoint,
        Some(model.into()),
        None,
    )));
    for stream in [false, true] {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let result = service.start_translation(
            "qa_live",
            TranslationRequest {
                text: "The meeting starts at nine tomorrow morning.".into(),
                from: "en-US".into(),
                to: "zh-CN".into(),
                stream,
            },
            Arc::new(move |id, event| {
                let _ = tx.send((id, event));
            }),
        );
        let id = result.unwrap_or_else(|e| panic!("{}", e.to_string().replace(key, "[redacted]")));
        let mut accumulated = String::new();
        let translated = tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                let (event_id, event) = rx.recv().await.expect("Translation ended without Done");
                assert_eq!(event_id, id);
                match event {
                    TranslationStreamEvent::Chunk(text) => accumulated.push_str(&text),
                    TranslationStreamEvent::Done(text) => break text,
                    TranslationStreamEvent::Error(error) => {
                        panic!("{}", error.replace(key, "[redacted]"))
                    }
                }
            }
        })
        .await
        .expect("Live translation timed out");
        assert!(!translated.contains(key), "Unexpected secret in response");
        assert!(!accumulated.contains(key), "Unexpected secret in stream");
        assert!(translated.contains("会议"), "Missing translated meeting");
        assert!(
            translated.contains('九') || translated.contains('9'),
            "Missing translated time"
        );
        assert!(!translated.contains('\u{fffd}'), "Broken UTF-8");
        assert_eq!(accumulated, translated);
        println!("Live custom provider stream={stream}: {translated}");
    }
    credentials.clear();
}
