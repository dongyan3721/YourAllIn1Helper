//! `mod-script-runner` —— 子功能 06：自定义脚本收录与一键执行。
//!
//! 设计要点：
//! - 解释器路径全部来自 `EnvProbeService`，缺失时明确返回不可用；
//! - 脚本内容写入隔离运行时文件，参数以 argv 数组传递，不拼接 shell 命令字符串；
//! - 内联/文件脚本、默认参数、工作目录、超时、单脚本快捷键、托盘显示、执行记录；
//! - stdout/stderr 增量推送，内存日志有上限；支持取消、超时与停用清理；
//! - `AIO_CLIPBOARD`/`AIO_SELECTION` 只能由显式注入的 `ScriptEnvironment` 提供；
//! - 用户脚本仅在点击运行或快捷键触发时执行，初始化与保存绝不执行。

pub mod command;
pub mod config;
pub mod env;
pub mod events;
pub mod hotkey;
pub mod model;
pub mod runner;
pub mod store;

use app_core::{
    AppModule, CoreError, CoreResult, ExternalToolKind, ModuleContext, ModuleMigration,
    ShortcutNormalizer, ShortcutSpec, TargetOs,
};
use config::{ScriptRunnerConfig, MAX_TIMEOUT_SECS, MODULE_ID};
use env::{ScriptEnvCapabilities, ScriptEnvironment};
use events::{NoopScriptEventSink, ScriptEventSink};
use hotkey::{NoopScriptHotkeySync, ScriptHotkeySync};
use model::{InterpreterStatus, LogEntry, ScriptRecord, ScriptRunRecord, ScriptType, ScriptUpsert};
use runner::Engine;
use std::sync::Arc;
use tracing::{info, warn};

pub const SCRIPT_ACTION_PREFIX: &str = "script:run:";
const MAX_SCRIPT_CONTENT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone)]
pub struct ScriptRunnerModule {
    engine: Arc<Engine>,
    hotkey_sync: Arc<dyn ScriptHotkeySync>,
}

impl ScriptRunnerModule {
    /// 无事件推送、无系统快捷键同步的模块实例；适合单元测试或非桌面宿主。
    pub fn new(environment: Arc<dyn ScriptEnvironment>) -> Self {
        Self::with_event_sink(environment, Arc::new(NoopScriptEventSink))
    }

    /// 显式注入桌面事件推送（Tauri `script://log-stream` 等），快捷键仅进入核心注册表。
    pub fn with_event_sink(
        environment: Arc<dyn ScriptEnvironment>,
        events: Arc<dyn ScriptEventSink>,
    ) -> Self {
        Self::with_services(environment, events, Arc::new(NoopScriptHotkeySync))
    }

    /// 生产宿主入口：显式注入事件推送与操作系统快捷键同步器。
    pub fn with_services(
        environment: Arc<dyn ScriptEnvironment>,
        events: Arc<dyn ScriptEventSink>,
        hotkey_sync: Arc<dyn ScriptHotkeySync>,
    ) -> Self {
        Self {
            engine: Arc::new(Engine::new(environment, events)),
            hotkey_sync,
        }
    }

    pub fn settings(&self, ctx: &ModuleContext) -> CoreResult<ScriptRunnerConfig> {
        ctx.module_settings::<ScriptRunnerConfig>(MODULE_ID)
            .map(ScriptRunnerConfig::normalized)
    }

    pub fn environment_capabilities(&self) -> ScriptEnvCapabilities {
        self.engine.environment().capabilities()
    }

    /// 探测所有支持类型的解释器；不支持的平台返回 supported=false。
    pub fn probe_interpreters(&self, ctx: &ModuleContext) -> Vec<InterpreterStatus> {
        ScriptType::ALL
            .into_iter()
            .map(|script_type| {
                probe_for(ctx, script_type).unwrap_or_else(|err| InterpreterStatus {
                    script_type,
                    supported: script_type.is_available_here(),
                    available: false,
                    resolved_path: None,
                    version_summary: None,
                    install_hint: String::new(),
                    unavailable_reason: Some(err.to_string()),
                })
            })
            .collect()
    }

    pub fn list_scripts(&self, ctx: &ModuleContext) -> CoreResult<Vec<ScriptRecord>> {
        let mut scripts = store::list_scripts(ctx)?;
        for script in &mut scripts {
            script.hotkey_active = script.hotkey.as_ref().is_some_and(|hotkey| {
                !hotkey.trim().is_empty()
                    && ctx
                        .shortcut_registry
                        .get_by_action(&hotkey_action_id(script.id))
                        .is_some()
            });
        }
        Ok(scripts)
    }

    pub fn get_script(&self, ctx: &ModuleContext, id: i64) -> CoreResult<Option<ScriptRecord>> {
        let mut script = store::get_script(ctx, id)?;
        if let Some(record) = &mut script {
            record.hotkey_active = record.hotkey.as_ref().is_some_and(|hotkey| {
                !hotkey.trim().is_empty()
                    && ctx
                        .shortcut_registry
                        .get_by_action(&hotkey_action_id(record.id))
                        .is_some()
            });
        }
        Ok(script)
    }

    /// 新建或更新脚本。只持久化，不执行脚本。
    pub fn save_script(
        &self,
        ctx: &ModuleContext,
        mut input: ScriptUpsert,
    ) -> CoreResult<ScriptRecord> {
        let config = self.settings(ctx)?;
        input.name = input.name.trim().to_string();
        if !input.is_inline {
            input.content_or_path = input.content_or_path.trim().to_string();
        }
        input.working_dir = input
            .working_dir
            .map(|dir| dir.trim().to_string())
            .filter(|dir| !dir.is_empty());
        input.hotkey = input
            .hotkey
            .map(|hotkey| hotkey.trim().to_string())
            .filter(|hotkey| !hotkey.is_empty());
        input.output_handling = input.output_handling.trim().to_ascii_lowercase();
        validate_input(&input)?;
        let timeout_secs = input
            .timeout_secs
            .unwrap_or(config.default_timeout_secs)
            .clamp(1, MAX_TIMEOUT_SECS);

        if let Some(id) = input.id {
            let previous = self
                .get_script(ctx, id)?
                .ok_or_else(|| CoreError::Config(format!("script {id} not found")))?;
            let action = hotkey_action_id(id);
            let previous_shortcut = ctx.shortcut_registry.get_by_action(&action);
            apply_hotkey_change(
                ctx,
                id,
                &input.name,
                previous.hotkey.as_deref(),
                input.hotkey.as_deref(),
            )?;
            match store::update_script(ctx, id, &input, timeout_secs) {
                Ok(record) => {
                    let desired = ctx.shortcut_registry.get_by_action(&action);
                    if let Err(error) = self.sync_hotkey_transition(
                        id,
                        previous_shortcut.as_ref(),
                        desired.as_ref(),
                    ) {
                        let mut message = format!("系统全局快捷键更新失败，脚本已回滚：{error}");
                        if let Err(restore_error) = store::restore_script(ctx, &previous) {
                            message.push_str(&format!("；数据库回滚失败：{restore_error}"));
                        }
                        if let Err(restore_error) = apply_hotkey_change(
                            ctx,
                            id,
                            &previous.name,
                            input.hotkey.as_deref(),
                            previous.hotkey.as_deref(),
                        ) {
                            message.push_str(&format!("；快捷键回滚失败：{restore_error}"));
                        }
                        return Err(CoreError::Config(message));
                    }
                    let mut record = record;
                    record.hotkey_active = record.hotkey.is_some() && desired.is_some();
                    Ok(record)
                }
                Err(err) => {
                    let _ = apply_hotkey_change(
                        ctx,
                        id,
                        &previous.name,
                        input.hotkey.as_deref(),
                        previous.hotkey.as_deref(),
                    );
                    Err(err)
                }
            }
        } else {
            let record = store::insert_script(ctx, &input, timeout_secs)?;
            let action = hotkey_action_id(record.id);
            if let Some(hotkey) = input.hotkey.as_deref() {
                if let Err(err) = register_hotkey(ctx, record.id, &record.name, hotkey) {
                    let _ = store::delete_script(ctx, record.id);
                    return Err(err);
                }
            }
            let desired = ctx.shortcut_registry.get_by_action(&action);
            if let Err(error) = self.sync_hotkey_transition(record.id, None, desired.as_ref()) {
                let _ = store::delete_script(ctx, record.id);
                ctx.shortcut_registry.unregister(&action);
                return Err(CoreError::Config(format!(
                    "系统全局快捷键更新失败，新建脚本已回滚：{error}"
                )));
            }
            self.get_script(ctx, record.id)?
                .ok_or_else(|| CoreError::Config("script disappeared after save".to_string()))
        }
    }

    pub fn delete_script(&self, ctx: &ModuleContext, id: i64) -> CoreResult<bool> {
        let action = hotkey_action_id(id);
        let previous = self.get_script(ctx, id)?;
        let previous_shortcut = ctx.shortcut_registry.get_by_action(&action);
        if previous.is_none() {
            return Ok(false);
        }
        if previous_shortcut.is_some() {
            ctx.shortcut_registry.unregister(&action);
        }
        match store::delete_script(ctx, id) {
            Ok(false) => {
                if let Some(shortcut) = previous_shortcut.as_ref() {
                    let _ = register_hotkey(
                        ctx,
                        id,
                        &previous.as_ref().expect("checked above").name,
                        &shortcut.active_logical_hotkey,
                    );
                }
                Ok(false)
            }
            Ok(true) => {
                let desired = ctx.shortcut_registry.get_by_action(&action);
                if let Err(error) =
                    self.sync_hotkey_transition(id, previous_shortcut.as_ref(), desired.as_ref())
                {
                    let record = previous.as_ref().expect("checked above");
                    let mut message = format!("系统全局快捷键更新失败，删除已回滚：{error}");
                    if let Err(restore_error) = store::restore_script(ctx, record) {
                        message.push_str(&format!("；数据库回滚失败：{restore_error}"));
                    }
                    if let Some(hotkey) = record.hotkey.as_deref() {
                        if let Err(restore_error) =
                            register_hotkey(ctx, record.id, &record.name, hotkey)
                        {
                            message.push_str(&format!("；快捷键回滚失败：{restore_error}"));
                        }
                    }
                    return Err(CoreError::Config(message));
                }
                Ok(true)
            }
            Err(err) => {
                if let Some(record) = previous {
                    if let Some(hotkey) = record.hotkey.as_deref() {
                        let _ = register_hotkey(ctx, record.id, &record.name, hotkey);
                    }
                }
                Err(err)
            }
        }
    }

    fn sync_hotkey_transition(
        &self,
        id: i64,
        previous: Option<&app_core::RegisteredShortcut>,
        desired: Option<&app_core::RegisteredShortcut>,
    ) -> CoreResult<()> {
        let changed = match (previous, desired) {
            (None, None) => false,
            (Some(previous), Some(desired)) => {
                previous.physical_accelerator != desired.physical_accelerator
            }
            _ => true,
        };
        if !changed {
            return Ok(());
        }
        self.hotkey_sync
            .sync(&hotkey_action_id(id), previous, desired)
            .map_err(CoreError::Config)
    }

    /// 仅在明确调用（IPC 点击、快捷键）时启动脚本。
    pub fn execute(
        &self,
        ctx: &ModuleContext,
        script_id: i64,
        extra_args: Vec<String>,
    ) -> CoreResult<ScriptRunRecord> {
        let script = self
            .get_script(ctx, script_id)?
            .ok_or_else(|| CoreError::Config(format!("script {script_id} not found")))?;
        let config = self.settings(ctx)?;
        self.engine.execute(
            ctx,
            &script,
            &extra_args,
            script.timeout_secs,
            config.max_log_bytes,
            config.keep_run_logs,
        )
    }

    pub fn kill(&self, run_id: &str) -> bool {
        self.engine.kill(run_id)
    }

    pub fn get_run(
        &self,
        ctx: &ModuleContext,
        run_id: &str,
    ) -> CoreResult<Option<ScriptRunRecord>> {
        self.engine.get_run(ctx, run_id)
    }

    pub fn list_runs(
        &self,
        ctx: &ModuleContext,
        script_id: Option<i64>,
        limit: usize,
    ) -> CoreResult<Vec<ScriptRunRecord>> {
        store::list_runs(ctx, script_id, limit)
    }

    pub fn get_logs(
        &self,
        ctx: &ModuleContext,
        run_id: &str,
        since_seq: u64,
    ) -> CoreResult<Option<(Vec<LogEntry>, ScriptRunRecord)>> {
        self.engine.get_logs(ctx, run_id, since_seq)
    }

    pub fn active_run_count(&self) -> usize {
        self.engine.active_count()
    }

    fn register_existing_hotkeys(&self, ctx: &ModuleContext) -> CoreResult<usize> {
        let scripts = store::list_scripts(ctx)?;
        let mut registered = 0;
        for script in scripts {
            let Some(hotkey) = script
                .hotkey
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            else {
                continue;
            };
            match register_hotkey(ctx, script.id, &script.name, hotkey) {
                Ok(()) => registered += 1,
                Err(err) => warn!(
                    script_id = script.id,
                    hotkey,
                    %err,
                    "Skipped invalid or conflicting script hotkey during init"
                ),
            }
        }
        Ok(registered)
    }

    fn unregister_all_hotkeys(&self, ctx: &ModuleContext) {
        match store::list_scripts(ctx) {
            Ok(scripts) => {
                for script in scripts {
                    ctx.shortcut_registry
                        .unregister(&hotkey_action_id(script.id));
                }
            }
            Err(err) => warn!(%err, "Failed to list scripts while unregistering hotkeys"),
        }
    }
}

pub fn hotkey_action_id(id: i64) -> String {
    format!("{SCRIPT_ACTION_PREFIX}{id}")
}

pub fn probe_for(ctx: &ModuleContext, script_type: ScriptType) -> CoreResult<InterpreterStatus> {
    let target_os = TargetOs::current();
    if !script_type.supported_on(target_os) {
        return Ok(InterpreterStatus {
            script_type,
            supported: false,
            available: false,
            resolved_path: None,
            version_summary: None,
            install_hint: String::new(),
            unavailable_reason: Some(format!(
                "{} scripts are not supported on this operating system",
                script_type.as_str()
            )),
        });
    }

    if script_type == ScriptType::Batch {
        #[cfg(target_os = "windows")]
        {
            let cmd = std::env::var_os("COMSPEC")
                .map(std::path::PathBuf::from)
                .filter(|path| path.is_file())
                .or_else(|| ctx.env_probe.find_in_system_path("cmd"));
            return Ok(match cmd {
                Some(path) => InterpreterStatus {
                    script_type,
                    supported: true,
                    available: true,
                    resolved_path: Some(path.to_string_lossy().into_owned()),
                    version_summary: Some("cmd.exe".to_string()),
                    install_hint: "Built into Windows".to_string(),
                    unavailable_reason: None,
                },
                None => InterpreterStatus {
                    script_type,
                    supported: true,
                    available: false,
                    resolved_path: None,
                    version_summary: None,
                    install_hint: "cmd.exe is built into Windows but COMSPEC was not found"
                        .to_string(),
                    unavailable_reason: Some("cmd.exe was not found".to_string()),
                },
            });
        }
        #[cfg(not(target_os = "windows"))]
        {
            return Ok(InterpreterStatus {
                script_type,
                supported: false,
                available: false,
                resolved_path: None,
                version_summary: None,
                install_hint: "Batch scripts are only available on Windows".to_string(),
                unavailable_reason: Some("Batch scripts are only available on Windows".to_string()),
            });
        }
    }

    let tool = match script_type {
        ScriptType::Python => ExternalToolKind::Python3,
        ScriptType::Node => ExternalToolKind::Node,
        ScriptType::Shell => ExternalToolKind::Zsh,
        ScriptType::AppleScript => ExternalToolKind::Osascript,
        ScriptType::PowerShell => ExternalToolKind::PowerShell,
        ScriptType::Vbscript => ExternalToolKind::Cscript,
        ScriptType::Batch => unreachable!("batch handled above"),
    };
    let probe = ctx.env_probe.probe(tool, false);
    Ok(InterpreterStatus {
        script_type,
        supported: true,
        available: probe.available,
        resolved_path: probe
            .resolved_path
            .map(|path| path.to_string_lossy().into_owned()),
        version_summary: probe.version_summary,
        install_hint: probe.install_hint,
        unavailable_reason: if probe.available {
            None
        } else {
            Some(format!(
                "{} interpreter was not found",
                script_type.as_str()
            ))
        },
    })
}

fn validate_input(input: &ScriptUpsert) -> CoreResult<()> {
    if input.name.is_empty() {
        return Err(CoreError::Config("script name is required".to_string()));
    }
    if input.name.chars().count() > 200 {
        return Err(CoreError::Config("script name is too long".to_string()));
    }
    if input.content_or_path.trim().is_empty() {
        return Err(CoreError::Config(
            "script content or file path is required".to_string(),
        ));
    }
    if input.is_inline && input.content_or_path.len() > MAX_SCRIPT_CONTENT_BYTES {
        return Err(CoreError::Config(format!(
            "inline script content exceeds {MAX_SCRIPT_CONTENT_BYTES} bytes"
        )));
    }
    if !matches!(
        input.output_handling.as_str(),
        "stream" | "final" | "discard"
    ) {
        return Err(CoreError::Config(
            "output_handling must be one of stream, final, discard".to_string(),
        ));
    }
    if let Some(timeout) = input.timeout_secs {
        if !(1..=MAX_TIMEOUT_SECS).contains(&timeout) {
            return Err(CoreError::Config(format!(
                "timeout_secs must be between 1 and {MAX_TIMEOUT_SECS}"
            )));
        }
    }
    if let Some(hotkey) = input.hotkey.as_deref() {
        ShortcutNormalizer::normalize_for_os(hotkey, TargetOs::current())?;
    }
    Ok(())
}

fn register_hotkey(ctx: &ModuleContext, id: i64, name: &str, hotkey: &str) -> CoreResult<()> {
    let spec = ShortcutSpec::new(
        MODULE_ID,
        hotkey_action_id(id),
        hotkey,
        format!("运行脚本 {name}"),
    );
    ctx.shortcut_registry.register(spec, None).map(|_| ())
}

fn apply_hotkey_change(
    ctx: &ModuleContext,
    id: i64,
    name: &str,
    previous: Option<&str>,
    next: Option<&str>,
) -> CoreResult<()> {
    let action = hotkey_action_id(id);
    if previous == next {
        if next.is_some() && ctx.shortcut_registry.get_by_action(&action).is_none() {
            if let Some(hotkey) = next {
                register_hotkey(ctx, id, name, hotkey)?;
            }
        }
        return Ok(());
    }
    match next {
        Some(hotkey) => register_hotkey(ctx, id, name, hotkey),
        None => {
            ctx.shortcut_registry.unregister(&action);
            Ok(())
        }
    }
}

impl AppModule for ScriptRunnerModule {
    fn id(&self) -> &'static str {
        MODULE_ID
    }

    fn name(&self) -> &'static str {
        "自定义脚本"
    }

    fn migrations(&self) -> Vec<ModuleMigration> {
        store::migrations()
    }

    fn default_shortcuts(&self) -> Vec<ShortcutSpec> {
        Vec::new()
    }

    fn on_init(&self, ctx: &ModuleContext) -> CoreResult<()> {
        self.engine.start_running();
        match store::mark_stale_runs_interrupted(ctx) {
            Ok(count) if count > 0 => {
                info!(count, "Marked stale running script records as interrupted")
            }
            Ok(_) => {}
            Err(err) => warn!(%err, "Failed to mark stale script runs"),
        }
        let runtime_dir = ctx.app_data_dir.join("script_runtime");
        if runtime_dir.exists() {
            if let Err(err) = std::fs::remove_dir_all(&runtime_dir) {
                warn!(%err, path = %runtime_dir.display(), "Failed to clean stale script runtime files");
            }
        }
        let registered = self.register_existing_hotkeys(ctx)?;
        info!(
            registered_script_hotkeys = registered,
            "脚本模块初始化完成（未执行任何用户脚本）"
        );
        Ok(())
    }

    fn on_shortcut(&self, action_id: &str, ctx: &ModuleContext) -> CoreResult<()> {
        let Some(raw_id) = action_id.strip_prefix(SCRIPT_ACTION_PREFIX) else {
            return Ok(());
        };
        let id: i64 = raw_id
            .parse()
            .map_err(|_| CoreError::Config(format!("invalid script action: {action_id}")))?;
        self.execute(ctx, id, Vec::new())?;
        Ok(())
    }

    fn on_shutdown(&self, ctx: &ModuleContext) -> CoreResult<()> {
        self.engine.shutdown();
        self.unregister_all_hotkeys(ctx);
        Ok(())
    }
}

pub use env::UnavailableScriptEnvironment;

#[cfg(test)]
#[path = "../tests/unit/lib.rs"]
mod tests;
