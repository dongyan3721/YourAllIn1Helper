//! 单脚本快捷键的系统同步接口。
//!
//! 业务模块只维护 `ShortcutRegistry`；真正注册到操作系统由宿主注入的同步器完成。
//! 同步器必须在失败时尽力恢复 previous 的系统注册，不能静默成功或破坏已有快捷键。
use app_core::RegisteredShortcut;

pub trait ScriptHotkeySync: Send + Sync + 'static {
    /// 将单个脚本动作从 `previous` 迁移到 `desired`。
    ///
    /// - `previous == desired` 时实现应直接返回成功；
    /// - 任一参数为 `None` 表示新增或删除；
    /// - 返回 `Err` 时，实现应保证 previous 的系统注册仍然有效，或已完成恢复。
    fn sync(
        &self,
        action_id: &str,
        previous: Option<&RegisteredShortcut>,
        desired: Option<&RegisteredShortcut>,
    ) -> Result<(), String>;
}

/// 无桌面宿主的缺省实现：核心注册表仍工作，但不触碰操作系统全局快捷键。
#[derive(Debug, Default)]
pub struct NoopScriptHotkeySync;

impl ScriptHotkeySync for NoopScriptHotkeySync {
    fn sync(
        &self,
        _action_id: &str,
        _previous: Option<&RegisteredShortcut>,
        _desired: Option<&RegisteredShortcut>,
    ) -> Result<(), String> {
        Ok(())
    }
}
