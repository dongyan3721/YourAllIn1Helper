//! 脚本库、解释器状态与执行记录的数据模型。
use app_core::TargetOs;
use serde::{Deserialize, Serialize};

/// 当前支持的脚本类型；命令构造与解释器探测均以该枚举为准。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptType {
    Python,
    Node,
    Shell,
    #[serde(rename = "applescript", alias = "apple_script")]
    AppleScript,
    #[serde(rename = "powershell", alias = "power_shell")]
    PowerShell,
    Batch,
    Vbscript,
}

impl ScriptType {
    pub const ALL: [ScriptType; 7] = [
        Self::Python,
        Self::Node,
        Self::Shell,
        Self::AppleScript,
        Self::PowerShell,
        Self::Batch,
        Self::Vbscript,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::Node => "node",
            Self::Shell => "shell",
            Self::AppleScript => "applescript",
            Self::PowerShell => "powershell",
            Self::Batch => "batch",
            Self::Vbscript => "vbscript",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|item| item.as_str() == value)
    }

    /// 生成内联脚本文件扩展名；不依赖用户输入，避免路径穿越。
    pub fn extension(self) -> &'static str {
        match self {
            Self::Python => "py",
            Self::Node => "js",
            Self::Shell => "sh",
            Self::AppleScript => "applescript",
            Self::PowerShell => "ps1",
            Self::Batch => "cmd",
            Self::Vbscript => "vbs",
        }
    }

    /// 该脚本类型是否在指定操作系统上受支持。
    pub fn supported_on(self, os: TargetOs) -> bool {
        match self {
            Self::Python | Self::Node => true,
            Self::Shell | Self::AppleScript => !matches!(os, TargetOs::Windows),
            Self::PowerShell | Self::Batch | Self::Vbscript => matches!(os, TargetOs::Windows),
        }
    }

    pub fn is_available_here(self) -> bool {
        self.supported_on(TargetOs::current())
    }
}

impl Default for ScriptType {
    fn default() -> Self {
        Self::Shell
    }
}

/// 解释器探测结果，直接来自 `EnvProbeService`，不伪造缺失能力。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterpreterStatus {
    pub script_type: ScriptType,
    pub supported: bool,
    pub available: bool,
    pub resolved_path: Option<String>,
    pub version_summary: Option<String>,
    pub install_hint: String,
    pub unavailable_reason: Option<String>,
}

/// 脚本库记录（`custom_scripts` 表）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptRecord {
    pub id: i64,
    pub name: String,
    pub script_type: ScriptType,
    pub is_inline: bool,
    pub content_or_path: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub working_dir: Option<String>,
    pub hotkey: Option<String>,
    pub show_in_tray: bool,
    pub timeout_secs: u64,
    pub output_handling: String,
    pub created_at: i64,
    pub updated_at: i64,
    /// 由注册表实时计算，不写入数据库；用于界面准确显示快捷键是否真正生效。
    #[serde(default)]
    pub hotkey_active: bool,
}

/// 新建/更新脚本的输入。`id = None` 表示新建。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScriptUpsert {
    pub id: Option<i64>,
    pub name: String,
    pub script_type: ScriptType,
    pub is_inline: bool,
    pub content_or_path: String,
    pub args: Vec<String>,
    pub working_dir: Option<String>,
    pub hotkey: Option<String>,
    pub show_in_tray: bool,
    pub timeout_secs: Option<u64>,
    pub output_handling: String,
}

impl Default for ScriptUpsert {
    fn default() -> Self {
        Self {
            id: None,
            name: String::new(),
            script_type: ScriptType::default(),
            is_inline: true,
            content_or_path: String::new(),
            args: Vec::new(),
            working_dir: None,
            hotkey: None,
            show_in_tray: false,
            timeout_secs: None,
            output_handling: "stream".to_string(),
        }
    }
}

/// 脚本执行状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
    SpawnFailed,
    Interrupted,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
            Self::SpawnFailed => "spawn_failed",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn from_db_str(value: &str) -> Self {
        match value {
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "timed_out" => Self::TimedOut,
            "cancelled" => Self::Cancelled,
            "spawn_failed" => Self::SpawnFailed,
            "interrupted" => Self::Interrupted,
            _ => Self::Running,
        }
    }

    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Running)
    }
}

/// stdout/stderr 流标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogStream {
    Stdout,
    Stderr,
}

impl LogStream {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}

/// 增量日志条目。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    pub seq: u64,
    pub stream: LogStream,
    pub text: String,
    pub at: i64,
}

/// 持久化执行记录（`script_run_logs` 表）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptRunRecord {
    pub run_id: String,
    pub script_id: i64,
    pub script_name: String,
    pub status: RunStatus,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub pid: Option<u32>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub duration_ms: Option<u64>,
    pub stdout_tail: String,
    pub stderr_tail: String,
    pub error_message: Option<String>,
    pub log_bytes: usize,
    pub dropped_bytes: usize,
}

/// 完成事件摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSummary {
    pub run_id: String,
    pub script_id: i64,
    pub script_name: String,
    pub status: RunStatus,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub pid: Option<u32>,
    pub started_at: i64,
    pub finished_at: i64,
    pub duration_ms: u64,
    pub log_bytes: usize,
    pub dropped_bytes: usize,
}

impl ScriptRunRecord {
    pub fn summary(&self) -> RunSummary {
        RunSummary {
            run_id: self.run_id.clone(),
            script_id: self.script_id,
            script_name: self.script_name.clone(),
            status: self.status,
            exit_code: self.exit_code,
            timed_out: self.timed_out,
            pid: self.pid,
            started_at: self.started_at,
            finished_at: self.finished_at.unwrap_or(self.started_at),
            duration_ms: self.duration_ms.unwrap_or_default(),
            log_bytes: self.log_bytes,
            dropped_bytes: self.dropped_bytes,
        }
    }
}
