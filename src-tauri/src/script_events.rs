//! 将脚本模块的增量日志与完成事件桥接为 Tauri 事件。
use mod_script_runner::events::ScriptEventSink;
use mod_script_runner::model::{LogEntry, RunSummary};
use serde_json::json;
use tauri::{AppHandle, Emitter};

pub struct TauriScriptEventSink {
    app: AppHandle,
}

impl TauriScriptEventSink {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl ScriptEventSink for TauriScriptEventSink {
    fn on_log(&self, run_id: &str, entry: &LogEntry) {
        let _ = self.app.emit(
            "script://log-stream",
            json!({ "run_id": run_id, "entry": entry }),
        );
    }

    fn on_finished(&self, summary: &RunSummary) {
        let _ = self.app.emit("script://execution-finished", summary);
        let _ = self.app.emit("kernel://updated", serde_json::Value::Null);
    }
}
