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

use app_core::{
    AppModule, CoreError, CoreResult, ModuleContext, ShortcutSpec, WindowSpec,
};
use config::TranslatorConfig;
use credentials::{CredentialStorage, SessionCredentialStore};
use os_hal::{HalError, HalResult, SelectionExtractor, SelectionTextResult};
use parking_lot::RwLock;
use provider::{openai::OpenAIProvider, TranslationRequest};
use service::{StreamEventSink, TranslatorService};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub struct TranslatorModule {
    selection_extractor: Arc<dyn SelectionExtractor>,
    service: Arc<TranslatorService>,
    config: Arc<RwLock<TranslatorConfig>>,
    event_sink: Arc<RwLock<Option<StreamEventSink>>>,
    is_active: Arc<AtomicBool>,
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
        }
    }

    /// 设置前端流式事件桥接下发回调。
    pub fn set_event_sink(&self, sink: StreamEventSink) {
        *self.event_sink.write() = Some(sink);
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

        self.service
            .execute_translation(target_provider, req, sink)
            .await
    }

    /// 取消当前正在执行的翻译。
    pub async fn cancel_current_translation(&self) {
        self.service.cancel_current().await;
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

        // 1. 严禁先弹窗抢占焦点！先通过无障碍提取前台选区
        let selection_opt = match self.extract_selected_text() {
            Ok(s) => s,
            Err(HalError::PermissionDenied { capability, guidance }) => {
                // 权限拒绝：唤起气泡展示权限指引
                let spec = WindowSpec::translate_popover(300.0, 200.0);
                let _ = ctx.window_pool.open_window(spec);
                return Err(CoreError::Internal(anyhow::anyhow!(
                    "未获得无障碍权限 [{}]: {}",
                    capability,
                    guidance
                )));
            }
            Err(HalError::Unsupported(_)) => {
                // 不支持该应用的选区提取
                None
            }
            Err(e) => {
                return Err(CoreError::Internal(anyhow::anyhow!(
                    "选区读取失败: {}",
                    e
                )));
            }
        };

        // 2. 唤起气泡窗口（位置可由前端后续通过鼠标位置重定位）
        let spec = WindowSpec::translate_popover(300.0, 200.0);
        ctx.window_pool.open_window(spec)?;

        // 3. 若成功读取到文本，触发翻译
        if let Some(selection) = selection_opt {
            let this_module = self.is_active.clone();
            let cfg = self.config();
            let service = self.service.clone();
            let sink = self.event_sink.read().clone().unwrap_or_else(|| Arc::new(|_, _| {}));

            tokio::spawn(async move {
                if !this_module.load(Ordering::SeqCst) {
                    return;
                }
                let req = TranslationRequest {
                    text: selection.text,
                    from: cfg.source_lang,
                    to: cfg.target_lang,
                    stream: true,
                };
                let _ = service
                    .execute_translation(&cfg.default_provider_id, req, sink)
                    .await;
            });
        }

        Ok(())
    }

    fn on_shutdown(&self, ctx: &ModuleContext) -> CoreResult<()> {
        self.is_active.store(false, Ordering::SeqCst);

        // 取消正在运行的翻译任务
        let service = self.service.clone();
        tokio::spawn(async move {
            service.cancel_current().await;
        });

        // 销毁浮动气泡窗口，防止孤儿残留
        let _ = ctx.window_pool.destroy_window("translate-popover");

        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests/unit/lib.rs"]
mod tests;

