//! OpenAI 兼容大模型翻译 API 适配器（支持原生 SSE 流式输出与 UTF-8 跨块安全解析）。
//!
//! 适配标准：
//! - Endpoint: https://api.openai.com/v1/chat/completions 或任意第三方/本地大模型兼容端点
//! - Header: Authorization: Bearer {KEY}
//! - Body: {"model": "...", "messages": [...], "stream": true, "temperature": 0.3}

use super::{TranslationProvider, TranslationRequest, TranslationStreamEvent};
use crate::stream::{parse_sse_openai_delta, Utf8ChunkBuffer};
use app_core::{CoreError, CoreResult};
use serde_json::json;
use std::pin::Pin;

pub struct OpenAIProvider {
    id: String,
    name: String,
    endpoint: String,
    model: String,
    temperature: f32,
}

impl Default for OpenAIProvider {
    fn default() -> Self {
        Self {
            id: "openai".to_string(),
            name: "OpenAI / 大模型 API (兼容网关)".to_string(),
            endpoint: "https://api.openai.com/v1/chat/completions".to_string(),
            model: "gpt-4o-mini".to_string(),
            temperature: 0.3,
        }
    }
}

impl OpenAIProvider {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn custom(
        id: impl Into<String>,
        name: impl Into<String>,
        endpoint: impl Into<String>,
        model: Option<String>,
        temperature: Option<f32>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            endpoint: endpoint.into(),
            model: model.unwrap_or_else(|| "gpt-4o-mini".to_string()),
            temperature: temperature.unwrap_or(0.3),
        }
    }

    fn system_prompt(to_lang: &str) -> String {
        let target_desc = match to_lang {
            "zh-CN" | "zh" => "简体中文 (Simplified Chinese)",
            "en-US" | "en" => "English",
            "ja-JP" | "ja" => "日本語 (Japanese)",
            _ => "Simplified Chinese",
        };
        format!(
            "You are a professional, accurate translator. Translate the given text directly into {}. Output ONLY the translated text, without commentary, markdown wrappers, or greetings.",
            target_desc
        )
    }

    fn is_local_endpoint(&self) -> bool {
        self.endpoint.contains("localhost")
            || self.endpoint.contains("127.0.0.1")
            || self.endpoint.contains("0.0.0.0")
    }
}

impl TranslationProvider for OpenAIProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn translate(
        &self,
        client: &reqwest::Client,
        req: &TranslationRequest,
        api_key: Option<&str>,
    ) -> Pin<Box<dyn std::future::Future<Output = CoreResult<String>> + Send>> {
        let key = api_key.unwrap_or("").trim().to_string();
        if key.is_empty() && !self.is_local_endpoint() {
            return Box::pin(async {
                Err(CoreError::Config(
                    "OpenAI API Key 未配置，请在「设置 → 翻译服务」中填写凭据".to_string(),
                ))
            });
        }

        let endpoint = self.endpoint.clone();
        let model = self.model.clone();
        let temperature = self.temperature;
        let system_msg = Self::system_prompt(&req.to);
        let user_text = req.text.clone();
        let client = client.clone();

        Box::pin(async move {
            let payload = json!({
                "model": model,
                "messages": [
                    { "role": "system", "content": system_msg },
                    { "role": "user", "content": user_text }
                ],
                "stream": false,
                "temperature": temperature
            });

            let mut req_builder = client
                .post(&endpoint)
                .header("Content-Type", "application/json")
                .json(&payload);

            if !key.is_empty() {
                req_builder = req_builder.header("Authorization", format!("Bearer {}", key));
            }

            let resp = req_builder
                .send()
                .await
                .map_err(|e| CoreError::Internal(anyhow::anyhow!("OpenAI API 请求失败: {}", e)))?;

            let status = resp.status();
            if !status.is_success() {
                let err_text = resp.text().await.unwrap_or_default();
                return Err(CoreError::Internal(anyhow::anyhow!(
                    "OpenAI API 返回错误 [HTTP {}]: {}",
                    status,
                    err_text
                )));
            }

            let body: serde_json::Value = resp.json().await.map_err(|e| {
                CoreError::Internal(anyhow::anyhow!("解析 OpenAI 响应 JSON 失败: {}", e))
            })?;

            let content = body
                .get("choices")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("message"))
                .and_then(|m| m.get("content"))
                .and_then(|s| s.as_str())
                .ok_or_else(|| {
                    CoreError::Internal(anyhow::anyhow!("OpenAI 响应中缺失 content 字段"))
                })?;

            Ok(content.to_string())
        })
    }

    fn translate_stream(
        &self,
        client: &reqwest::Client,
        req: &TranslationRequest,
        api_key: Option<&str>,
        tx: tokio::sync::mpsc::Sender<TranslationStreamEvent>,
    ) -> Pin<Box<dyn std::future::Future<Output = CoreResult<()>> + Send>> {
        let key = api_key.unwrap_or("").trim().to_string();
        if key.is_empty() && !self.is_local_endpoint() {
            let tx = tx.clone();
            return Box::pin(async move {
                let err_msg =
                    "OpenAI API Key 未配置，请在「设置 → 翻译服务」中填写凭据".to_string();
                let _ = tx
                    .send(TranslationStreamEvent::Error(err_msg.clone()))
                    .await;
                Err(CoreError::Config(err_msg))
            });
        }

        let endpoint = self.endpoint.clone();
        let model = self.model.clone();
        let temperature = self.temperature;
        let system_msg = Self::system_prompt(&req.to);
        let user_text = req.text.clone();
        let client = client.clone();

        Box::pin(async move {
            let payload = json!({
                "model": model,
                "messages": [
                    { "role": "system", "content": system_msg },
                    { "role": "user", "content": user_text }
                ],
                "stream": true,
                "temperature": temperature
            });

            let mut req_builder = client
                .post(&endpoint)
                .header("Content-Type", "application/json")
                .header("Accept", "text/event-stream")
                .json(&payload);

            if !key.is_empty() {
                req_builder = req_builder.header("Authorization", format!("Bearer {}", key));
            }

            let resp = match req_builder.send().await {
                Ok(r) => r,
                Err(e) => {
                    let err = format!("OpenAI SSE 请求建立失败: {}", e);
                    let _ = tx.send(TranslationStreamEvent::Error(err.clone())).await;
                    return Err(CoreError::Internal(anyhow::anyhow!(err)));
                }
            };

            let status = resp.status();
            if !status.is_success() {
                let err_text = resp.text().await.unwrap_or_default();
                let err = format!("OpenAI SSE 响应错误 [HTTP {}]: {}", status, err_text);
                let _ = tx.send(TranslationStreamEvent::Error(err.clone())).await;
                return Err(CoreError::Internal(anyhow::anyhow!(err)));
            }

            let mut resp = resp;
            let mut utf8_buffer = Utf8ChunkBuffer::new();
            let mut full_accumulated = String::new();
            let mut line_buffer = String::new();

            loop {
                let chunk_opt = match resp.chunk().await {
                    Ok(opt) => opt,
                    Err(e) => {
                        let err = format!("SSE 接收流分片网络错误: {}", e);
                        let _ = tx.send(TranslationStreamEvent::Error(err.clone())).await;
                        return Err(CoreError::Internal(anyhow::anyhow!(err)));
                    }
                };

                let chunk = match chunk_opt {
                    Some(c) => c,
                    None => break,
                };

                // UTF-8 跨块安全解析
                let text_piece = utf8_buffer.push_bytes(&chunk);
                line_buffer.push_str(&text_piece);

                while let Some(pos) = line_buffer.find('\n') {
                    let line = line_buffer[..pos].to_string();
                    line_buffer.drain(..=pos);

                    if let Some(delta) = parse_sse_openai_delta(&line) {
                        if !delta.is_empty() {
                            full_accumulated.push_str(&delta);
                            let _ = tx.send(TranslationStreamEvent::Chunk(delta)).await;
                        }
                    }
                }
            }

            // 处理尾部剩余缓冲
            let trailing = utf8_buffer.finish();
            if !trailing.is_empty() {
                line_buffer.push_str(&trailing);
            }
            if let Some(delta) = parse_sse_openai_delta(&line_buffer) {
                if !delta.is_empty() {
                    full_accumulated.push_str(&delta);
                    let _ = tx.send(TranslationStreamEvent::Chunk(delta)).await;
                }
            }

            let _ = tx
                .send(TranslationStreamEvent::Done(full_accumulated))
                .await;
            Ok(())
        })
    }
}
