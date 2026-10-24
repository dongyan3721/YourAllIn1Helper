//! 功能自身配置：日志内存上限、默认超时与执行记录保留条数。
use serde::{Deserialize, Serialize};

pub const MODULE_ID: &str = "script_runner";
pub const MIN_LOG_BYTES: usize = 4 * 1024;
pub const MAX_LOG_BYTES: usize = 4 * 1024 * 1024;
pub const MIN_KEEP_RUNS: usize = 10;
pub const MAX_KEEP_RUNS: usize = 1000;
pub const MAX_TIMEOUT_SECS: u64 = 86_400;

/// 持久化于 `modules.settings.script_runner` 的配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScriptRunnerConfig {
    /// 单次运行在内存中保留的 stdout+stderr 字节上限，防止无限增长。
    pub max_log_bytes: usize,
    /// 新建脚本的默认超时（秒）。
    pub default_timeout_secs: u64,
    /// 执行记录保留条数，超出后删除最旧记录。
    pub keep_run_logs: usize,
}

impl Default for ScriptRunnerConfig {
    fn default() -> Self {
        Self {
            max_log_bytes: 128 * 1024,
            default_timeout_secs: 60,
            keep_run_logs: 100,
        }
    }
}

impl ScriptRunnerConfig {
    /// 校验并夹紧不可信输入，避免配置写入非法值后拖垮运行。
    pub fn normalized(mut self) -> Self {
        self.max_log_bytes = self.max_log_bytes.clamp(MIN_LOG_BYTES, MAX_LOG_BYTES);
        self.default_timeout_secs = self.default_timeout_secs.clamp(1, MAX_TIMEOUT_SECS);
        self.keep_run_logs = self.keep_run_logs.clamp(MIN_KEEP_RUNS, MAX_KEEP_RUNS);
        self
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(MIN_LOG_BYTES..=MAX_LOG_BYTES).contains(&self.max_log_bytes) {
            return Err(format!(
                "max_log_bytes must be between {MIN_LOG_BYTES} and {MAX_LOG_BYTES}"
            ));
        }
        if !(1..=MAX_TIMEOUT_SECS).contains(&self.default_timeout_secs) {
            return Err(format!(
                "default_timeout_secs must be between 1 and {MAX_TIMEOUT_SECS}"
            ));
        }
        if !(MIN_KEEP_RUNS..=MAX_KEEP_RUNS).contains(&self.keep_run_logs) {
            return Err(format!(
                "keep_run_logs must be between {MIN_KEEP_RUNS} and {MAX_KEEP_RUNS}"
            ));
        }
        Ok(())
    }
}
