//! 翻译调度服务核心：管理异步请求、取消旧任务与结果顺序防覆盖。
//!
//! 关键机制：
//! 1. 单调递增请求 ID (request_id)：每次发起新翻译，原子递增 ID。
//! 2. 迟到结果隔离：流式回调与完成事件均校验 request_id，凡旧请求分片一律丢弃，绝不覆盖新结果。
//! 3. 异步非阻塞：所有网络 I/O 运行在 Tokio 异步协程中，AppKit 主事件循环不阻塞。
//! 4. 协作式与抢占式取消：新请求进入或用户取消时，立即 abort 上一个正在执行的后台协程。

use crate::{
    credentials::CredentialStorage,
    provider::{
        deepl::DeepLProvider, google::GoogleProvider, openai::OpenAIProvider, TranslationProvider,
        TranslationRequest, TranslationStreamEvent,
    },
};
use app_core::{CoreError, CoreResult};
use parking_lot::RwLock;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use tokio::sync::Mutex as TokioMutex;

/// 事件下发回调类型：(request_id, event)
pub type StreamEventSink = Arc<dyn Fn(u64, TranslationStreamEvent) + Send + Sync + 'static>;

pub struct TranslatorService {
    http_client: reqwest::Client,
    providers: Arc<RwLock<HashMap<String, Arc<dyn TranslationProvider>>>>,
    credentials: Arc<dyn CredentialStorage>,
    current_request_id: Arc<AtomicU64>,
    active_task: Arc<TokioMutex<Option<tokio::task::JoinHandle<()>>>>,
}

impl TranslatorService {
    pub fn new(credentials: Arc<dyn CredentialStorage>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .no_proxy()
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let mut map: HashMap<String, Arc<dyn TranslationProvider>> = HashMap::new();
        let google = Arc::new(GoogleProvider::new());
        let deepl = Arc::new(DeepLProvider::new());
        let openai = Arc::new(OpenAIProvider::new());

        map.insert(google.id().to_string(), google);
        map.insert(deepl.id().to_string(), deepl);
        map.insert(openai.id().to_string(), openai);

        Self {
            http_client: client,
            providers: Arc::new(RwLock::new(map)),
            credentials,
            current_request_id: Arc::new(AtomicU64::new(0)),
            active_task: Arc::new(TokioMutex::new(None)),
        }
    }

    /// 注册或替换服务提供商（用于测试注入 local fixture 或用户配置自定义大模型接口）。
    pub fn register_provider(&self, provider: Arc<dyn TranslationProvider>) {
        self.providers
            .write()
            .insert(provider.id().to_string(), provider);
    }

    /// 列出所有已注册提供商的基本元数据（ID、名称、是否需要 Key，不包含任何密钥明文）。
    pub fn list_providers_metadata(&self) -> Vec<serde_json::Value> {
        let providers = self.providers.read();
        let mut list = Vec::new();
        for p in providers.values() {
            let has_key = self.credentials.has_api_key(p.id());
            list.push(serde_json::json!({
                "id": p.id(),
                "name": p.name(),
                "requires_api_key": p.requires_api_key(),
                "has_api_key": has_key,
            }));
        }
        list.sort_by(|a, b| {
            let id_a = a["id"].as_str().unwrap_or_default();
            let id_b = b["id"].as_str().unwrap_or_default();
            id_a.cmp(id_b)
        });
        list
    }

    /// 获取指定提供商实例。
    pub fn get_provider(&self, id: &str) -> Option<Arc<dyn TranslationProvider>> {
        self.providers.read().get(id).cloned()
    }

    /// 获取凭据存储句柄。
    pub fn credentials(&self) -> Arc<dyn CredentialStorage> {
        self.credentials.clone()
    }

    /// 取消当前正在进行的翻译任务。
    pub async fn cancel_current(&self) {
        let mut lock = self.active_task.lock().await;
        if let Some(task) = lock.take() {
            task.abort();
        }
        // 递增 request_id，彻底隔离所有在网络队列中的迟到包
        self.current_request_id.fetch_add(1, Ordering::SeqCst);
    }

    /// 发起翻译（支持流式响应），返回生成的单调请求 ID。
    pub async fn execute_translation(
        &self,
        provider_id: &str,
        req: TranslationRequest,
        event_sink: StreamEventSink,
    ) -> CoreResult<u64> {
        // 1. 校验文本非空
        if req.text.trim().is_empty() {
            return Err(CoreError::Config("待翻译文本不能为空".to_string()));
        }

        // 2. 取消旧任务并生成新的 request_id
        let req_id = self.current_request_id.fetch_add(1, Ordering::SeqCst) + 1;
        {
            let mut lock = self.active_task.lock().await;
            if let Some(task) = lock.take() {
                task.abort();
            }
        }

        // 3. 解析目标 Provider
        let provider = self
            .get_provider(provider_id)
            .ok_or_else(|| CoreError::Config(format!("未找到指定的翻译服务源: {}", provider_id)))?;

        // 4. 从安全凭据库获取 API Key（内存安全）
        let api_key = self.credentials.get_api_key(provider.id())?;

        // 5. 在后台协程中执行，不阻塞主调用线程
        let client = self.http_client.clone();
        let current_id_tracker = self.current_request_id.clone();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<TranslationStreamEvent>(64);

        // 桥接接收协程：严格检查迟到结果
        let sink_clone = event_sink.clone();
        let id_tracker_clone = current_id_tracker.clone();
        let forwarder = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                // 如果当前全局 ID 已发生改变，说明已有新请求发起或被取消，丢弃迟到分片
                if id_tracker_clone.load(Ordering::SeqCst) != req_id {
                    break;
                }
                sink_clone(req_id, event);
            }
        });

        // 核心翻译协程
        let handle = tokio::spawn(async move {
            let key_ref = api_key.as_deref();
            if req.stream {
                let _ = provider
                    .translate_stream(&client, &req, key_ref, tx)
                    .await;
            } else {
                match provider.translate(&client, &req, key_ref).await {
                    Ok(translated) => {
                        let _ = tx
                            .send(TranslationStreamEvent::Chunk(translated.clone()))
                            .await;
                        let _ = tx.send(TranslationStreamEvent::Done(translated)).await;
                    }
                    Err(err) => {
                        let _ = tx
                            .send(TranslationStreamEvent::Error(err.to_string()))
                            .await;
                    }
                }
                drop(tx);
            }
            let _ = forwarder.await;
        });

        let mut lock = self.active_task.lock().await;
        *lock = Some(handle);

        Ok(req_id)
    }
}
