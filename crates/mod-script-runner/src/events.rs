//! 日志与完成事件的显式注入点；桌面主程序用 Tauri 事件实现，测试可用内存实现。
use crate::model::{LogEntry, RunSummary};

pub trait ScriptEventSink: Send + Sync + 'static {
    fn on_log(&self, _run_id: &str, _entry: &LogEntry) {}
    fn on_finished(&self, _summary: &RunSummary) {}
}

/// 缺省空实现，避免模块依赖具体 UI 框架。
#[derive(Debug, Default)]
pub struct NoopScriptEventSink;

impl ScriptEventSink for NoopScriptEventSink {}
