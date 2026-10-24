//! 显式注入的 AIO_CLIPBOARD / AIO_SELECTION 能力。
//!
//! 业务模块不直接依赖 `NativeOsHal`；主程序提供真实适配器。没有实现的能力必须返回
//! `Unsupported`，执行时不会设置对应环境变量，也不会用测试数据伪造。
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptEnvCapabilities {
    pub clipboard: bool,
    pub selection: bool,
    pub detail: String,
}

impl Default for ScriptEnvCapabilities {
    fn default() -> Self {
        Self {
            clipboard: false,
            selection: false,
            detail: "No script environment adapter injected".to_string(),
        }
    }
}

pub trait ScriptEnvironment: Send + Sync + 'static {
    fn capabilities(&self) -> ScriptEnvCapabilities;

    fn clipboard_text(&self) -> Result<Option<String>, String> {
        Err("AIO_CLIPBOARD is not supported by the injected environment".to_string())
    }

    fn selected_text(&self) -> Result<Option<String>, String> {
        Err("AIO_SELECTION is not supported by the injected environment".to_string())
    }
}

/// 无注入能力时的安全缺省值：不读取用户剪贴板，不伪造选区文本。
#[derive(Debug, Default)]
pub struct UnavailableScriptEnvironment;

impl ScriptEnvironment for UnavailableScriptEnvironment {
    fn capabilities(&self) -> ScriptEnvCapabilities {
        ScriptEnvCapabilities::default()
    }
}
