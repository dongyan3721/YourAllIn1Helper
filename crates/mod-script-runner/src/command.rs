//! 纯命令构造：脚本内容只写入文件，执行参数以数组传递，绝不拼接 shell 命令字符串。
use crate::model::ScriptType;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
}

/// 根据脚本类型构造解释器参数。该函数不接触 shell 字符串：
/// - 脚本路径和每个用户参数都是独立的 argv 项，空格、引号不会被二次解释；
/// - Batch 使用 `cmd.exe /d /c <script> <args...>`，脚本路径仍是独立参数；
/// - Windows 由调用方附加 `CREATE_NO_WINDOW`。
pub fn build_command(
    script_type: ScriptType,
    interpreter: PathBuf,
    script_path: &str,
    args: &[String],
) -> ExecutionCommand {
    let script = script_path.to_string();
    let mut argv: Vec<String> = Vec::new();
    match script_type {
        ScriptType::Python => {
            argv.push("-u".to_string());
            argv.push(script);
        }
        ScriptType::Node => {
            argv.push(script);
        }
        ScriptType::Shell | ScriptType::AppleScript => {
            argv.push(script);
        }
        ScriptType::PowerShell => {
            argv.extend([
                "-NoProfile".to_string(),
                "-NonInteractive".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-File".to_string(),
                script,
            ]);
        }
        ScriptType::Batch => {
            argv.extend(["/d".to_string(), "/c".to_string(), script]);
        }
        ScriptType::Vbscript => {
            argv.extend(["//NoLogo".to_string(), "//B".to_string(), script]);
        }
    }
    argv.extend(args.iter().cloned());
    ExecutionCommand {
        program: interpreter,
        args: argv,
    }
}

pub fn inline_file_name(script_type: ScriptType) -> String {
    format!("script.{}", script_type.extension())
}

/// 应用平台特定执行选项：Unix 独立进程组便于连同子进程一起终止，Windows 隐藏控制台。
pub fn apply_platform_options(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
    }
}
