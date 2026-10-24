//! 翻译服务提供商（Gateway）接口契约与多源适配器。
//!
//! 规范要求：
//! 1. 适配官方 Google Cloud Translation、DeepL 与 OpenAI 兼容大模型 API。
//! 2. 缺 API Key 时如实向用户提示未配置，严禁找寻用户系统现存凭据，严禁假冒免费默认服务。
//! 3. 生产环境禁止 Mock 回退。

pub mod deepl;
pub mod google;
pub mod openai;

use app_core::CoreResult;
use serde::{Deserialize, Serialize};

/// 翻译请求载荷。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationRequest {
    pub text: String,
    pub from: String,
    pub to: String,
    pub stream: bool,
}

/// 流式翻译事件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum TranslationStreamEvent {
    Chunk(String),
    Done(String),
    Error(String),
}

/// 服务提供商核心抽象 Trait。
pub trait TranslationProvider: Send + Sync + 'static {
    /// 唯一标识（例如 "google", "deepl", "openai" 或用户自定义 ID）
    fn id(&self) -> &str;

    /// 友好显示名称
    fn name(&self) -> &str;

    /// 是否强制需要 API Key
    fn requires_api_key(&self) -> bool {
        true
    }

    /// 执行同步（单次返回）翻译
    fn translate(
        &self,
        client: &reqwest::Client,
        req: &TranslationRequest,
        api_key: Option<&str>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = CoreResult<String>> + Send>>;

    /// 执行 SSE 流式翻译，通过 mpsc::Sender 推送片段与完成事件
    fn translate_stream(
        &self,
        client: &reqwest::Client,
        req: &TranslationRequest,
        api_key: Option<&str>,
        tx: tokio::sync::mpsc::Sender<TranslationStreamEvent>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = CoreResult<()>> + Send>>;
}

/// 将统一语言标识（如 "zh-CN", "en-US", "ja-JP"）规范化映射至各服务商所需代码。
pub fn normalize_language_code(lang: &str) -> &'static str {
    match lang.trim() {
        "zh" | "zh-CN" | "zh-Hans" | "chinese" => "zh-CN",
        "en" | "en-US" | "english" => "en-US",
        "ja" | "ja-JP" | "japanese" => "ja-JP",
        _ => "auto",
    }
}
