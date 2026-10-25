//! 官方 DeepL REST API 适配器。
//!
//! 官方规范：
//! - Free API: https://api-free.deepl.com/v2/translate (key 以 :fx 结尾)
//! - Pro API: https://api.deepl.com/v2/translate
//! - Header: Authorization: DeepL-Auth-Key {KEY}
//! - Body: {"text": ["..."], "target_lang": "ZH", "source_lang": "EN"}

use super::{TranslationProvider, TranslationRequest, TranslationStreamEvent};
use app_core::{CoreError, CoreResult};
use serde_json::json;
use std::pin::Pin;

pub struct DeepLProvider {
    custom_endpoint: Option<String>,
}

impl Default for DeepLProvider {
    fn default() -> Self {
        Self {
            custom_endpoint: None,
        }
    }
}

impl DeepLProvider {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_endpoint(endpoint: impl Into<String>) -> Self {
        Self {
            custom_endpoint: Some(endpoint.into()),
        }
    }

    fn resolve_endpoint(&self, api_key: &str) -> String {
        if let Some(ref ep) = self.custom_endpoint {
            return ep.clone();
        }
        if api_key.ends_with(":fx") {
            "https://api-free.deepl.com/v2/translate".to_string()
        } else {
            "https://api.deepl.com/v2/translate".to_string()
        }
    }

    fn map_target_lang(lang: &str) -> &'static str {
        match lang {
            "zh-CN" | "zh" => "ZH",
            "en-US" | "en" => "EN-US",
            "ja-JP" | "ja" => "JA",
            _ => "ZH",
        }
    }

    fn map_source_lang(lang: &str) -> Option<&'static str> {
        match lang {
            "zh-CN" | "zh" => Some("ZH"),
            "en-US" | "en" => Some("EN"),
            "ja-JP" | "ja" => Some("JA"),
            _ => None,
        }
    }
}

impl TranslationProvider for DeepLProvider {
    fn id(&self) -> &str {
        "deepl"
    }

    fn name(&self) -> &str {
        "DeepL (官方 API)"
    }

    fn translate(
        &self,
        client: &reqwest::Client,
        req: &TranslationRequest,
        api_key: Option<&str>,
    ) -> Pin<Box<dyn std::future::Future<Output = CoreResult<String>> + Send>> {
        let key = match api_key {
            Some(k) if !k.trim().is_empty() => k.to_string(),
            _ => {
                return Box::pin(async {
                    Err(CoreError::Config(
                        "DeepL API Key 未配置，请在「设置 → 翻译服务」中填写官方凭据".to_string(),
                    ))
                });
            }
        };

        let endpoint = self.resolve_endpoint(&key);
        let text = req.text.clone();
        let target = Self::map_target_lang(&req.to);
        let source = Self::map_source_lang(&req.from);
        let client = client.clone();

        Box::pin(async move {
            let mut payload = json!({
                "text": [text],
                "target_lang": target,
            });
            if let Some(src) = source {
                payload["source_lang"] = json!(src);
            }

            let resp = client
                .post(&endpoint)
                .header("Authorization", format!("DeepL-Auth-Key {}", key))
                .header("Content-Type", "application/json")
                .json(&payload)
                .send()
                .await
                .map_err(|e| CoreError::Internal(anyhow::anyhow!("DeepL API 请求失败: {}", e)))?;

            let status = resp.status();
            if !status.is_success() {
                let err_text = resp.text().await.unwrap_or_default();
                return Err(CoreError::Internal(anyhow::anyhow!(
                    "DeepL API 返回错误 [HTTP {}]: {}",
                    status,
                    err_text
                )));
            }

            let body: serde_json::Value = resp.json().await.map_err(|e| {
                CoreError::Internal(anyhow::anyhow!("解析 DeepL 响应 JSON 失败: {}", e))
            })?;

            let translated = body
                .get("translations")
                .and_then(|t| t.get(0))
                .and_then(|first| first.get("text"))
                .and_then(|s| s.as_str())
                .ok_or_else(|| {
                    CoreError::Internal(anyhow::anyhow!("DeepL 响应中缺失 text 字段"))
                })?;

            Ok(translated.to_string())
        })
    }

    fn translate_stream(
        &self,
        client: &reqwest::Client,
        req: &TranslationRequest,
        api_key: Option<&str>,
        tx: tokio::sync::mpsc::Sender<TranslationStreamEvent>,
    ) -> Pin<Box<dyn std::future::Future<Output = CoreResult<()>> + Send>> {
        let fut = self.translate(client, req, api_key);
        Box::pin(async move {
            match fut.await {
                Ok(text) => {
                    let _ = tx.send(TranslationStreamEvent::Chunk(text.clone())).await;
                    let _ = tx.send(TranslationStreamEvent::Done(text)).await;
                    Ok(())
                }
                Err(err) => {
                    let _ = tx
                        .send(TranslationStreamEvent::Error(err.to_string()))
                        .await;
                    Err(err)
                }
            }
        })
    }
}
