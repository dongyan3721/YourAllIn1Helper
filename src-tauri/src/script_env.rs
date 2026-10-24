//! 生产环境的脚本注入能力：真实读取系统剪贴板与已注入的选区提取器。
use mod_script_runner::env::{ScriptEnvCapabilities, ScriptEnvironment};
use os_hal::SelectionExtractor;
use std::sync::Arc;

pub struct AioScriptEnvironment {
    selection: Arc<dyn SelectionExtractor>,
}

impl AioScriptEnvironment {
    pub fn new(selection: Arc<dyn SelectionExtractor>) -> Self {
        Self { selection }
    }
}

impl ScriptEnvironment for AioScriptEnvironment {
    fn capabilities(&self) -> ScriptEnvCapabilities {
        let clipboard = arboard::Clipboard::new().is_ok();
        let selection = self.selection.is_available();
        let detail = match (clipboard, selection) {
            (true, true) => "system clipboard + accessibility selection",
            (true, false) => {
                "system clipboard available; selection extraction is unsupported on this adapter"
            }
            (false, true) => "clipboard unavailable; accessibility selection available",
            (false, false) => "clipboard and selection are unavailable",
        };
        ScriptEnvCapabilities {
            clipboard,
            selection,
            detail: detail.to_string(),
        }
    }

    fn clipboard_text(&self) -> Result<Option<String>, String> {
        let mut clipboard =
            arboard::Clipboard::new().map_err(|error| format!("无法打开系统剪贴板: {error}"))?;
        clipboard
            .get_text()
            .map(Some)
            .map_err(|error| format!("无法读取系统剪贴板文本: {error}"))
    }

    fn selected_text(&self) -> Result<Option<String>, String> {
        if !self.selection.is_available() {
            return Err("当前系统未实现选区文字提取".to_string());
        }
        self.selection
            .extract_selected_text()
            .map(|selection| selection.map(|value| value.text))
            .map_err(|error| error.to_string())
    }
}
