//! 官方 Google Cloud Translation API v2 适配器。
//!
//! 官方规范：
//! - Endpoint: https://translation.googleapis.com/language/translate/v2
//! - Header: X-goog-api-key: {KEY} 或 Authorization: Bearer {KEY}
//! - Body: {"q": ["..."], "target": "zh", "source": "en", "format": "text"}
//! - 无凭据时返回准确提示，禁止静默调用非官方免费接口。

use super::{TranslationProvider, TranslationRequest, TranslationStreamEvent};
use app_core::{CoreError, CoreResult};
use serde_json::json;
use std::pin::Pin;

pub struct GoogleProvider {
    endpoint: String,
}

impl Default for GoogleProvider {
    fn default() -> Self {
        Self {
            endpoint: "https://translation.googleapis.com/language/translate/v2".to_string(),
        }
    }
}

impl GoogleProvider {
    pub fn new() -> Self {
        Self::default()
    }

    /// 支持在测试中替换为本地 HTTP fixture endpoint。
    pub fn with_endpoint(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
        }
    }

    fn map_lang(lang: &str) -> &'static str {
        match lang {
            "zh-CN" | "zh" => "zh",
            "en-US" | "en" => "en",
            "ja-JP" | "ja" => "ja",
            _ => "",
        }
    }
}

impl TranslationProvider for GoogleProvider {
    fn id(&self) -> &str {
        "google"
    }

    fn name(&self) -> &str {
        "Google 翻译 (官方 API)"
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
                        "Google Translate API Key 未配置，请在「设置 → 翻译服务」中填写官方凭据"
                            .to_string(),
                    ))
                });
            }
        };

        let endpoint = self.endpoint.clone();
        let text = req.text.clone();
        let target = Self::map_lang(&req.to);
        let source = Self::map_lang(&req.from);
        let client = client.clone();

        Box::pin(async move {
            let mut payload = json!({
                "q": [text],
                "target": target,
                "format": "text"
            });
            if !source.is_empty() {
                payload["source"] = json!(source);
            }

            let resp = client
                .post(&endpoint)
                .header("X-goog-api-key", &key)
                .header("Content-Type", "application/json")
                .json(&payload)
                .send()
                .await
                .map_err(|e| CoreError::Internal(anyhow::anyhow!("Google API 请求失败: {}", e)))?;

            let status = resp.status();
            if !status.is_success() {
                let err_text = resp.text().await.unwrap_or_default();
                return Err(CoreError::Internal(anyhow::anyhow!(
                    "Google API 返回错误 [HTTP {}]: {}",
                    status,
                    err_text
                )));
            }

            let body: serde_json::Value = resp.json().await.map_err(|e| {
                CoreError::Internal(anyhow::anyhow!("解析 Google 响应 JSON 失败: {}", e))
            })?;

            let translated = body
                .get("data")
                .and_then(|d| d.get("translations"))
                .and_then(|t| t.get(0))
                .and_then(|first| first.get("translatedText"))
                .and_then(|s| s.as_str())
                .ok_or_else(|| {
                    CoreError::Internal(anyhow::anyhow!("Google 响应中缺失 translatedText 字段"))
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
