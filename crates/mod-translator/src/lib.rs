//! `mod-translator` — 全局划词翻译功能积木（Feature 07）。
//!
//! 职责：
//! 1. 领域驱动，仅依赖注入的 OS HAL Trait（`SelectionExtractor`）与凭据库接口（`CredentialStorage`）；
//! 2. 原生无障碍取词优先，提取前保持目标窗口焦点；
//! 3. 官方 Google / DeepL / OpenAI 兼容网关，支持 SSE 跨块 UTF-8 安全流式传输；
//! 4. 并发取消与结果防覆盖（monotonic sequence ID）；
//! 5. 生产禁止 Mock 回退，API Key 严禁明文落盘。

pub mod config;
pub mod credentials;
pub mod provider;
pub mod service;
pub mod stream;

use app_core::{AppModule, CoreError, CoreResult, ModuleContext, ShortcutSpec, WindowSpec};
use config::TranslatorConfig;
use credentials::{CredentialStorage, SessionCredentialStore};
use os_hal::{HalError, HalResult, SelectionExtractor, SelectionTextResult};
use parking_lot::RwLock;
use provider::{openai::OpenAIProvider, TranslationRequest};
use serde::Serialize;
use service::{StreamEventSink, TranslatorService};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

/// 快捷键在前台应用仍聚焦时捕获的状态，由气泡就绪后一次性消费。
#[derive(Clone, Serialize)]
pub struct SelectionInvocation {
    pub text: Option<String>,
    pub permission_denied: bool,
    pub guidance: Option<String>,
    pub unsupported: bool,
}

pub struct TranslatorModule {
    selection_extractor: Arc<dyn SelectionExtractor>,
    service: Arc<TranslatorService>,
    config: Arc<RwLock<TranslatorConfig>>,
    event_sink: Arc<RwLock<Option<StreamEventSink>>>,
    is_active: Arc<AtomicBool>,
    pending_selection: RwLock<Option<SelectionInvocation>>,
    selection_sink: RwLock<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl TranslatorModule {
    pub fn new(selection_extractor: Arc<dyn SelectionExtractor>) -> Self {
        let creds = Arc::new(SessionCredentialStore::new());
        Self::with_credentials(selection_extractor, creds)
    }

    pub fn with_credentials(
        selection_extractor: Arc<dyn SelectionExtractor>,
        credentials: Arc<dyn CredentialStorage>,
    ) -> Self {
        let service = Arc::new(TranslatorService::new(credentials));
        Self {
            selection_extractor,
            service,
            config: Arc::new(RwLock::new(TranslatorConfig::default())),
            event_sink: Arc::new(RwLock::new(None)),
            is_active: Arc::new(AtomicBool::new(false)),
            pending_selection: RwLock::new(None),
            selection_sink: RwLock::new(None),
        }
    }

    /// 设置前端流式事件桥接下发回调。
    pub fn set_event_sink(&self, sink: StreamEventSink) {
        *self.event_sink.write() = Some(sink);
    }

    pub fn set_selection_sink(&self, sink: Arc<dyn Fn() + Send + Sync>) {
        *self.selection_sink.write() = Some(sink);
    }

    pub fn take_selection(&self) -> Option<SelectionInvocation> {
        self.pending_selection.write().take()
    }

    /// 获取底层翻译调度服务。
    pub fn service(&self) -> Arc<TranslatorService> {
        self.service.clone()
    }

    /// 读取当前内存中的非敏感设置。
    pub fn config(&self) -> TranslatorConfig {
        self.config.read().clone()
    }

    /// 原子更新并清洗配置。
    pub fn update_config(&self, mut new_cfg: TranslatorConfig) -> CoreResult<()> {
        new_cfg.sanitize();
        let mut ids = std::collections::HashSet::new();
        for custom in &new_cfg.custom_providers {
            if ["google", "deepl", "openai"].contains(&custom.id.as_str())
                || !ids.insert(custom.id.clone())
            {
                return Err(CoreError::Config(
                    "自定义服务 ID 不可重复或覆盖内置服务".into(),
                ));
            }
            let url = reqwest::Url::parse(&custom.endpoint_url)
                .map_err(|_| CoreError::Config("无效的翻译端点 URL".into()))?;
            if url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() {
                return Err(CoreError::Config("翻译端点必须有主机且不得包含凭据".into()));
            }
            if url.scheme() != "https" && !provider::openai::is_loopback_url(&url) {
                return Err(CoreError::Config("远程翻译端点必须使用 HTTPS".into()));
            }
        }
        self.service.remove_custom_providers();
        // 动态加载自定义 providers
        for custom in &new_cfg.custom_providers {
            if custom.is_openai_compatible {
                let p = Arc::new(OpenAIProvider::custom(
                    &custom.id,
                    &custom.name,
                    &custom.endpoint_url,
                    custom.model.clone(),
                    custom.temperature,
                ));
                self.service.register_provider(p);
            }
        }
        *self.config.write() = new_cfg;
        Ok(())
    }

    /// 调用 OS HAL 提取当前前台应用的选中文本（保持目标前台焦点）。
    pub fn extract_selected_text(&self) -> HalResult<Option<SelectionTextResult>> {
        self.selection_extractor.extract_selected_text()
    }

    /// 发起翻译任务。
    pub async fn translate(
        &self,
        text: &str,
        from: Option<&str>,
        to: Option<&str>,
        provider_id: Option<&str>,
        stream: bool,
    ) -> CoreResult<u64> {
        self.start_translation(text, from, to, provider_id, stream)
    }

    pub fn start_translation(
        &self,
        text: &str,
        from: Option<&str>,
        to: Option<&str>,
        provider_id: Option<&str>,
        stream: bool,
    ) -> CoreResult<u64> {
        let cfg = self.config();
        let target_lang = to.unwrap_or(&cfg.target_lang).to_string();
        let source_lang = from.unwrap_or(&cfg.source_lang).to_string();
        let target_provider = provider_id.unwrap_or(&cfg.default_provider_id);

        let req = TranslationRequest {
            text: text.to_string(),
            from: source_lang,
            to: target_lang,
            stream,
        };

        let sink: StreamEventSink = match self.event_sink.read().clone() {
            Some(s) => s,
            None => Arc::new(|_id, _ev| {}),
        };

        self.service.start_translation(target_provider, req, sink)
    }

    /// 取消当前正在执行的翻译。
    pub fn cancel_current_translation(&self) {
        self.service.cancel_current();
    }

    /// 是否处于运行状态。
    pub fn is_active(&self) -> bool {
        self.is_active.load(Ordering::SeqCst)
    }
}

impl AppModule for TranslatorModule {
    fn id(&self) -> &'static str {
        "translator"
    }

    fn name(&self) -> &'static str {
        "全局划词翻译"
    }

    fn default_shortcuts(&self) -> Vec<ShortcutSpec> {
        let cfg = self.config();
        vec![ShortcutSpec::new(
            self.id(),
            "translator:translate_selection",
            &cfg.hotkey,
            "全局划词翻译",
        )]
    }

    fn on_init(&self, ctx: &ModuleContext) -> CoreResult<()> {
        // 尝试从模块设置加载配置
        if let Ok(settings) = ctx.module_settings::<TranslatorConfig>(self.id()) {
            let _ = self.update_config(settings);
        }
        self.is_active.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn on_shortcut(&self, action: &str, ctx: &ModuleContext) -> CoreResult<()> {
        if action != "translator:translate_selection" {
            return Ok(());
        }

        // 先取消旧请求并在第三方仍聚焦时捕获；不在同步快捷键钩子中启动 Tokio 协程。
        self.service.cancel_current();
        let mut invocation = SelectionInvocation {
            text: None,
            permission_denied: false,
            guidance: None,
            unsupported: false,
        };
        match self.extract_selected_text() {
            Ok(selection) => invocation.text = selection.map(|s| s.text),
            Err(HalError::PermissionDenied { guidance, .. }) => {
                invocation.permission_denied = true;
                invocation.guidance = Some(guidance);
            }
            Err(HalError::Unsupported(reason)) => {
                invocation.unsupported = true;
                invocation.guidance = Some(reason.into());
            }
            Err(e) => return Err(CoreError::Internal(anyhow::anyhow!("选区读取失败: {}", e))),
        }
        *self.pending_selection.write() = Some(invocation);
        if let Err(error) = ctx
            .window_pool
            .open_window(WindowSpec::translate_popover(300.0, 200.0))
        {
            self.pending_selection.write().take();
            return Err(error);
        }
        // 初次打开由 mounted 主动消费；已存在窗口由事件唤醒，消费操作保证不重复提交。
        if let Some(sink) = self.selection_sink.read().clone() {
            sink();
        }

        Ok(())
    }

    fn on_shutdown(&self, ctx: &ModuleContext) -> CoreResult<()> {
        // 取消正在运行的翻译任务（纯同步锁清理，无需依赖外部异步上下文）
        self.service.cancel_current();
        self.pending_selection.write().take();

        // 销毁浮动气泡窗口，防止孤儿残留
        ctx.window_pool.destroy_window("translate-popover")?;
        self.is_active.store(false, Ordering::SeqCst);

        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests/unit/lib.rs"]
mod tests;
