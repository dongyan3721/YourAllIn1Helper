//! 单元与集成测试：使用真实 shell 进程验证执行/取消/超时/日志，显式注入环境能力。
use super::*;
use crate::command::{build_command, inline_file_name};
use crate::env::{ScriptEnvCapabilities, ScriptEnvironment};
use crate::events::{NoopScriptEventSink, ScriptEventSink};
use crate::hotkey::ScriptHotkeySync;
use crate::model::{
    LogEntry, LogStream, RunStatus, RunSummary, ScriptRunRecord, ScriptType, ScriptUpsert,
};
use app_core::{AppKernel, AppKernelBuilder};
use parking_lot::Mutex;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

#[derive(Default)]
struct TestEnvironment {
    clipboard: Option<String>,
    selection: Option<String>,
    clipboard_error: bool,
}

impl ScriptEnvironment for TestEnvironment {
    fn capabilities(&self) -> ScriptEnvCapabilities {
        ScriptEnvCapabilities {
            clipboard: self.clipboard.is_some(),
            selection: self.selection.is_some(),
            detail: "test adapter".to_string(),
        }
    }

    fn clipboard_text(&self) -> Result<Option<String>, String> {
        if self.clipboard_error {
            return Err("clipboard denied".to_string());
        }
        Ok(self.clipboard.clone())
    }

    fn selected_text(&self) -> Result<Option<String>, String> {
        Ok(self.selection.clone())
    }
}

#[derive(Debug, PartialEq, Eq)]
struct SyncCall {
    action_id: String,
    previous: Option<String>,
    desired: Option<String>,
}

#[derive(Default)]
struct RecordingHotkeySync {
    fail: AtomicBool,
    calls: Mutex<Vec<SyncCall>>,
}

impl RecordingHotkeySync {
    fn set_failing(&self, failing: bool) {
        self.fail.store(failing, Ordering::SeqCst);
    }
}

impl ScriptHotkeySync for RecordingHotkeySync {
    fn sync(
        &self,
        action_id: &str,
        previous: Option<&app_core::RegisteredShortcut>,
        desired: Option<&app_core::RegisteredShortcut>,
    ) -> Result<(), String> {
        self.calls.lock().push(SyncCall {
            action_id: action_id.to_string(),
            previous: previous.map(|value| value.physical_accelerator.clone()),
            desired: desired.map(|value| value.physical_accelerator.clone()),
        });
        if self.fail.load(Ordering::SeqCst) {
            Err("injected native registration failure".to_string())
        } else {
            Ok(())
        }
    }
}

#[derive(Default)]
struct TestSink {
    logs: Mutex<Vec<(String, LogEntry)>>,
    finished: Mutex<Vec<RunSummary>>,
}

impl ScriptEventSink for TestSink {
    fn on_log(&self, run_id: &str, entry: &LogEntry) {
        self.logs.lock().push((run_id.to_string(), entry.clone()));
    }

    fn on_finished(&self, summary: &RunSummary) {
        self.finished.lock().push(summary.clone());
    }
}

fn module_with(environment: Arc<dyn ScriptEnvironment>) -> ScriptRunnerModule {
    ScriptRunnerModule::new(environment)
}

fn kernel_with(dir: &Path, module: Arc<ScriptRunnerModule>, in_memory: bool) -> AppKernel {
    AppKernelBuilder::new(dir)
        .with_in_memory_db(in_memory)
        .register_module_arc(module)
        .unwrap()
        .bootstrap()
        .unwrap()
}

fn upsert(name: &str, script_type: ScriptType, content: &str) -> ScriptUpsert {
    ScriptUpsert {
        name: name.to_string(),
        script_type,
        is_inline: true,
        content_or_path: content.to_string(),
        timeout_secs: Some(15),
        output_handling: "stream".to_string(),
        ..ScriptUpsert::default()
    }
}

fn wait_terminal(
    module: &ScriptRunnerModule,
    ctx: &ModuleContext,
    run_id: &str,
) -> ScriptRunRecord {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(record) = module.get_run(ctx, run_id).unwrap() {
            if record.status.is_terminal() {
                return record;
            }
        }
        assert!(
            Instant::now() < deadline,
            "run {run_id} did not finish before deadline"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn shell_or_skip(ctx: &ModuleContext) -> bool {
    match probe_for(ctx, ScriptType::Shell) {
        Ok(status) if status.available => true,
        _ => {
            eprintln!("Skipping shell execution test: /bin/zsh or /bin/bash is unavailable");
            false
        }
    }
}

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[test]
fn command_builder_uses_argument_arrays_for_every_supported_type() {
    let path = "/tmp/a b/script";
    let python = build_command(
        ScriptType::Python,
        PathBuf::from("/usr/bin/python3"),
        path,
        &args(&["hello world", "\"quoted\""]),
    );
    assert_eq!(python.program, PathBuf::from("/usr/bin/python3"));
    assert_eq!(python.args, vec!["-u", path, "hello world", "\"quoted\""]);

    let node = build_command(
        ScriptType::Node,
        PathBuf::from("node"),
        path,
        &args(&["x y"]),
    );
    assert_eq!(node.args, vec![path, "x y"]);

    let shell = build_command(ScriptType::Shell, PathBuf::from("/bin/zsh"), path, &[]);
    assert_eq!(shell.args, vec![path]);

    let apple = build_command(
        ScriptType::AppleScript,
        PathBuf::from("/usr/bin/osascript"),
        path,
        &args(&["a"]),
    );
    assert_eq!(apple.args, vec![path, "a"]);

    let ps = build_command(
        ScriptType::PowerShell,
        PathBuf::from("powershell.exe"),
        r"C:\a b\script.ps1",
        &args(&["one"]),
    );
    assert_eq!(
        ps.args,
        vec![
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            r"C:\a b\script.ps1",
            "one"
        ]
    );

    let batch = build_command(
        ScriptType::Batch,
        PathBuf::from("cmd.exe"),
        r"C:\a b\script.cmd",
        &args(&["two"]),
    );
    assert_eq!(batch.args, vec!["/d", "/c", r"C:\a b\script.cmd", "two"]);

    let vbs = build_command(
        ScriptType::Vbscript,
        PathBuf::from("cscript.exe"),
        r"C:\a b\script.vbs",
        &[],
    );
    assert_eq!(vbs.args, vec!["//NoLogo", "//B", r"C:\a b\script.vbs"]);
}

#[test]
fn inline_script_file_names_are_derived_from_the_type_only() {
    assert_eq!(inline_file_name(ScriptType::Python), "script.py");
    assert_eq!(inline_file_name(ScriptType::Node), "script.js");
    assert_eq!(inline_file_name(ScriptType::Shell), "script.sh");
    assert_eq!(
        inline_file_name(ScriptType::AppleScript),
        "script.applescript"
    );
    assert_eq!(inline_file_name(ScriptType::PowerShell), "script.ps1");
    assert_eq!(inline_file_name(ScriptType::Batch), "script.cmd");
    assert_eq!(inline_file_name(ScriptType::Vbscript), "script.vbs");
}

#[test]
fn script_crud_persists_across_restart_and_init_does_not_execute() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("must-not-exist-on-save.txt");
    let script = format!("touch '{}'\n", marker.display());
    let module = Arc::new(module_with(Arc::new(TestEnvironment::default())));
    let kernel = kernel_with(dir.path(), Arc::clone(&module), false);

    let saved = module
        .save_script(
            kernel.context(),
            upsert("persisted", ScriptType::Shell, &script),
        )
        .unwrap();
    assert_eq!(saved.name, "persisted");
    assert!(!marker.exists(), "saving a script must not execute it");

    kernel.set_module_enabled("script_runner", false).unwrap();
    kernel.shutdown().unwrap();
    drop(kernel);

    let module = Arc::new(module_with(Arc::new(TestEnvironment::default())));
    let kernel = kernel_with(dir.path(), Arc::clone(&module), false);
    let scripts = module.list_scripts(kernel.context()).unwrap();
    assert_eq!(scripts.len(), 1);
    assert_eq!(scripts[0].id, saved.id);
    assert_eq!(scripts[0].content_or_path, script);
    assert!(
        !marker.exists(),
        "initializing a module must not execute scripts"
    );
}

#[test]
fn execution_streams_stdout_stderr_and_records_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let sink = Arc::new(TestSink::default());
    let module = Arc::new(ScriptRunnerModule::with_event_sink(
        Arc::new(TestEnvironment::default()),
        sink.clone(),
    ));
    let kernel = kernel_with(dir.path(), Arc::clone(&module), true);
    let ctx = kernel.context();
    if !shell_or_skip(ctx) {
        return;
    }

    let saved = module
        .save_script(
            ctx,
            upsert(
                "stream",
                ScriptType::Shell,
                "printf 'out-你好\\n'; sleep 0.2; printf 'err-line\\n' >&2; exit 3\n",
            ),
        )
        .unwrap();
    let running = module.execute(ctx, saved.id, Vec::new()).unwrap();
    assert_eq!(running.status, RunStatus::Running);
    let record = wait_terminal(&module, ctx, &running.run_id);
    assert_eq!(record.status, RunStatus::Failed);
    assert_eq!(record.exit_code, Some(3));
    assert!(record.stdout_tail.contains("out-你好"));
    assert!(record.stderr_tail.contains("err-line"));
    assert!(!record.timed_out);

    let logs = sink.logs.lock();
    assert!(logs
        .iter()
        .any(|(_, entry)| entry.stream == LogStream::Stdout));
    assert!(logs
        .iter()
        .any(|(_, entry)| entry.stream == LogStream::Stderr));
    drop(logs);
    assert_eq!(sink.finished.lock().len(), 1);

    let listed = module.list_runs(ctx, Some(saved.id), 10).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].status, RunStatus::Failed);
}

#[test]
fn stored_and_runtime_arguments_preserve_spaces_and_quotes() {
    let dir = tempfile::tempdir().unwrap();
    let module = Arc::new(module_with(Arc::new(TestEnvironment::default())));
    let kernel = kernel_with(dir.path(), Arc::clone(&module), true);
    let ctx = kernel.context();
    if !shell_or_skip(ctx) {
        return;
    }

    let mut input = upsert("args", ScriptType::Shell, "printf '<%s>\\n' \"$@\"\n");
    input.args = args(&["hello world", "\"quoted\""]);
    let saved = module.save_script(ctx, input).unwrap();
    let running = module
        .execute(ctx, saved.id, args(&["runtime arg"]))
        .unwrap();
    let record = wait_terminal(&module, ctx, &running.run_id);
    assert_eq!(record.status, RunStatus::Succeeded);
    assert!(record.stdout_tail.contains("<hello world>"));
    assert!(record.stdout_tail.contains("<\u{22}quoted\u{22}>"));
    assert!(record.stdout_tail.contains("<runtime arg>"));
}

#[test]
fn file_script_with_spaces_runs_from_its_directory_and_is_not_modified() {
    let dir = tempfile::tempdir().unwrap();
    let script_dir = dir.path().join("my scripts");
    std::fs::create_dir_all(&script_dir).unwrap();
    let script_path = script_dir.join("hello world.sh");
    std::fs::write(
        &script_path,
        "printf 'cwd=%s\\narg=<%s>\\n' \"$PWD\" \"$1\"\n",
    )
    .unwrap();
    let original = std::fs::read_to_string(&script_path).unwrap();

    let module = Arc::new(module_with(Arc::new(TestEnvironment::default())));
    let kernel = kernel_with(dir.path(), Arc::clone(&module), true);
    let ctx = kernel.context();
    if !shell_or_skip(ctx) {
        return;
    }

    let mut input = upsert("file", ScriptType::Shell, &script_path.to_string_lossy());
    input.is_inline = false;
    let saved = module.save_script(ctx, input).unwrap();
    let running = module
        .execute(ctx, saved.id, args(&["value with spaces"]))
        .unwrap();
    let record = wait_terminal(&module, ctx, &running.run_id);
    assert_eq!(record.status, RunStatus::Succeeded);
    assert!(record.stdout_tail.contains("cwd="));
    assert!(record.stdout_tail.contains("arg=<value with spaces>"));
    assert_eq!(std::fs::read_to_string(&script_path).unwrap(), original);
}

#[test]
fn python_node_and_applescript_run_when_interpreters_are_available() {
    let dir = tempfile::tempdir().unwrap();
    let module = Arc::new(module_with(Arc::new(TestEnvironment::default())));
    let kernel = kernel_with(dir.path(), Arc::clone(&module), true);
    let ctx = kernel.context();

    let cases = [
        (
            ScriptType::Python,
            "python",
            "import sys\nprint('python', sys.argv[1])\nprint('py-err', file=sys.stderr)\n",
            "python ok",
        ),
        (
            ScriptType::Node,
            "node",
            "console.log('node', process.argv[2]);\nconsole.error('node-err');\n",
            "node ok",
        ),
        (
            ScriptType::AppleScript,
            "applescript",
            "on run argv\nreturn \"apple \" & item 1 of argv\nend run\n",
            "apple ok",
        ),
    ];

    for (script_type, name, content, expected) in cases {
        let status = probe_for(ctx, script_type).unwrap();
        if !status.available {
            eprintln!(
                "Skipping {name} execution test: interpreter unavailable ({:?})",
                status.unavailable_reason
            );
            continue;
        }
        let mut input = upsert(name, script_type, content);
        input.args = args(&["ok"]);
        let saved = module.save_script(ctx, input).unwrap();
        let running = module.execute(ctx, saved.id, Vec::new()).unwrap();
        let record = wait_terminal(&module, ctx, &running.run_id);
        assert_eq!(
            record.status,
            RunStatus::Succeeded,
            "{name} stderr: {}",
            record.stderr_tail
        );
        assert!(
            record.stdout_tail.contains(expected),
            "{name} stdout: {}",
            record.stdout_tail
        );
    }
}

#[test]
fn timeout_kills_sleeping_script_and_marks_timed_out() {
    let dir = tempfile::tempdir().unwrap();
    let module = Arc::new(module_with(Arc::new(TestEnvironment::default())));
    let kernel = kernel_with(dir.path(), Arc::clone(&module), true);
    let ctx = kernel.context();
    if !shell_or_skip(ctx) {
        return;
    }

    let mut input = upsert("timeout", ScriptType::Shell, "sleep 30\n");
    input.timeout_secs = Some(1);
    let saved = module.save_script(ctx, input).unwrap();
    let started = Instant::now();
    let running = module.execute(ctx, saved.id, Vec::new()).unwrap();
    let record = wait_terminal(&module, ctx, &running.run_id);
    assert_eq!(record.status, RunStatus::TimedOut);
    assert!(record.timed_out);
    assert!(started.elapsed() < Duration::from_secs(6));
    assert_eq!(module.active_run_count(), 0);
}

#[test]
fn kill_cancels_running_script_and_shutdown_cleans_up() {
    let dir = tempfile::tempdir().unwrap();
    let module = Arc::new(module_with(Arc::new(TestEnvironment::default())));
    let kernel = kernel_with(dir.path(), Arc::clone(&module), true);
    let ctx = kernel.context();
    if !shell_or_skip(ctx) {
        return;
    }

    let saved = module
        .save_script(ctx, upsert("sleep", ScriptType::Shell, "sleep 30\n"))
        .unwrap();
    let running = module.execute(ctx, saved.id, Vec::new()).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    assert!(module.kill(&running.run_id));
    let cancelled = wait_terminal(&module, ctx, &running.run_id);
    assert_eq!(cancelled.status, RunStatus::Cancelled);
    assert!(!module.kill(&running.run_id));

    let second = module.execute(ctx, saved.id, Vec::new()).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    kernel.set_module_enabled("script_runner", false).unwrap();
    let stopped = module.get_run(ctx, &second.run_id).unwrap().unwrap();
    assert_eq!(stopped.status, RunStatus::Cancelled);
    assert_eq!(module.active_run_count(), 0);
    assert!(!kernel.registry().is_enabled("script_runner"));
}

#[test]
fn environment_values_are_injected_only_when_the_adapter_reports_them() {
    let dir = tempfile::tempdir().unwrap();
    let env = Arc::new(TestEnvironment {
        clipboard: Some("clip 你好".to_string()),
        selection: Some("selected text".to_string()),
        clipboard_error: false,
    });
    let module = Arc::new(ScriptRunnerModule::new(env));
    let kernel = kernel_with(dir.path(), Arc::clone(&module), true);
    let ctx = kernel.context();
    if !shell_or_skip(ctx) {
        return;
    }

    let saved = module
        .save_script(
            ctx,
            upsert(
                "env",
                ScriptType::Shell,
                "printf 'clip=%s\\nsel=%s\\n' \"$AIO_CLIPBOARD\" \"$AIO_SELECTION\"\n",
            ),
        )
        .unwrap();
    let running = module.execute(ctx, saved.id, Vec::new()).unwrap();
    let record = wait_terminal(&module, ctx, &running.run_id);
    assert!(record.stdout_tail.contains("clip=clip 你好"));
    assert!(record.stdout_tail.contains("sel=selected text"));

    let unavailable_dir = tempfile::tempdir().unwrap();
    let module = Arc::new(ScriptRunnerModule::new(Arc::new(
        UnavailableScriptEnvironment,
    )));
    let kernel = kernel_with(unavailable_dir.path(), Arc::clone(&module), true);
    let ctx = kernel.context();
    let saved = module
        .save_script(
            ctx,
            upsert(
                "no-env",
                ScriptType::Shell,
                "if [ -n \"${AIO_CLIPBOARD+x}\" ]; then echo set; else echo unset; fi\n",
            ),
        )
        .unwrap();
    let running = module.execute(ctx, saved.id, Vec::new()).unwrap();
    let record = wait_terminal(&module, ctx, &running.run_id);
    assert!(record.stdout_tail.contains("unset"));
}

#[test]
fn final_output_mode_suppresses_live_entries_but_keeps_final_tail() {
    let dir = tempfile::tempdir().unwrap();
    let sink = Arc::new(TestSink::default());
    let module = Arc::new(ScriptRunnerModule::with_event_sink(
        Arc::new(TestEnvironment::default()),
        sink.clone(),
    ));
    let kernel = kernel_with(dir.path(), Arc::clone(&module), true);
    let ctx = kernel.context();
    if !shell_or_skip(ctx) {
        return;
    }

    let mut input = upsert(
        "final-only",
        ScriptType::Shell,
        "printf 'first\n'; sleep 0.4; printf 'second\n'
",
    );
    input.output_handling = "final".to_string();
    let saved = module.save_script(ctx, input).unwrap();
    let running = module.execute(ctx, saved.id, Vec::new()).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    let (entries, _) = module.get_logs(ctx, &running.run_id, 0).unwrap().unwrap();
    assert!(
        entries.is_empty(),
        "final mode must not stream live entries"
    );
    let record = wait_terminal(&module, ctx, &running.run_id);
    assert!(record.stdout_tail.contains("first"));
    assert!(record.stdout_tail.contains("second"));
    assert!(sink.logs.lock().is_empty());
}

#[test]
fn log_cache_is_bounded_even_when_script_writes_a_lot() {
    let dir = tempfile::tempdir().unwrap();
    let module = Arc::new(module_with(Arc::new(TestEnvironment::default())));
    let kernel = kernel_with(dir.path(), Arc::clone(&module), true);
    let ctx = kernel.context();
    if !shell_or_skip(ctx) {
        return;
    }

    kernel
        .update_config(|cfg| {
            cfg.modules.settings.insert(
                crate::config::MODULE_ID.to_string(),
                serde_json::json!({
                    "max_log_bytes": 4096,
                    "default_timeout_secs": 30,
                    "keep_run_logs": 20
                }),
            );
        })
        .unwrap();

    let saved = module
        .save_script(
            ctx,
            upsert(
                "noisy",
                ScriptType::Shell,
                "i=0; while [ $i -lt 400 ]; do printf '0123456789abcdef'; i=$((i+1)); done\n",
            ),
        )
        .unwrap();
    let running = module.execute(ctx, saved.id, Vec::new()).unwrap();
    let record = wait_terminal(&module, ctx, &running.run_id);
    assert_eq!(record.status, RunStatus::Succeeded);
    assert!(record.log_bytes <= 4096);
    assert!(record.dropped_bytes > 0);
}

#[test]
fn hotkeys_follow_module_lifecycle_and_conflicts_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let module = Arc::new(module_with(Arc::new(TestEnvironment::default())));
    let kernel = kernel_with(dir.path(), Arc::clone(&module), true);
    let ctx = kernel.context();
    if !shell_or_skip(ctx) {
        return;
    }

    let mut input = upsert("hotkey", ScriptType::Shell, "printf 'hotkey-ran\\n'\n");
    input.hotkey = Some("PrimaryMod+Alt+F9".to_string());
    let saved = module.save_script(ctx, input).unwrap();
    assert!(saved.hotkey_active);
    assert!(ctx
        .shortcut_registry
        .get_by_action(&hotkey_action_id(saved.id))
        .is_some());

    kernel.trigger_action(&hotkey_action_id(saved.id)).unwrap();
    let runs = module.list_runs(ctx, Some(saved.id), 10).unwrap();
    assert_eq!(runs.len(), 1);
    let record = wait_terminal(&module, ctx, &runs[0].run_id);
    assert!(record.stdout_tail.contains("hotkey-ran"));

    let mut conflicting = upsert("conflict", ScriptType::Shell, "echo nope\n");
    conflicting.hotkey = Some("PrimaryMod+Alt+F9".to_string());
    assert!(module.save_script(ctx, conflicting).is_err());
    assert_eq!(module.list_scripts(ctx).unwrap().len(), 1);

    kernel.set_module_enabled("script_runner", false).unwrap();
    assert!(ctx
        .shortcut_registry
        .get_by_action(&hotkey_action_id(saved.id))
        .is_none());
    kernel.set_module_enabled("script_runner", true).unwrap();
    assert!(ctx
        .shortcut_registry
        .get_by_action(&hotkey_action_id(saved.id))
        .is_some());
}

#[test]
fn delete_removes_script_and_releases_hotkey_but_keeps_run_history() {
    let dir = tempfile::tempdir().unwrap();
    let module = Arc::new(module_with(Arc::new(TestEnvironment::default())));
    let kernel = kernel_with(dir.path(), Arc::clone(&module), true);
    let ctx = kernel.context();
    if !shell_or_skip(ctx) {
        return;
    }

    let mut input = upsert("delete-me", ScriptType::Shell, "echo deleted\n");
    input.hotkey = Some("PrimaryMod+Alt+F10".to_string());
    let saved = module.save_script(ctx, input).unwrap();
    let running = module.execute(ctx, saved.id, Vec::new()).unwrap();
    wait_terminal(&module, ctx, &running.run_id);
    assert!(module.delete_script(ctx, saved.id).unwrap());
    assert!(module.list_scripts(ctx).unwrap().is_empty());
    assert!(ctx
        .shortcut_registry
        .get_by_action(&hotkey_action_id(saved.id))
        .is_none());
    let runs = module.list_runs(ctx, None, 10).unwrap();
    assert_eq!(runs.len(), 1);
}

#[test]
fn incremental_logs_have_monotonic_sequence_numbers() {
    let dir = tempfile::tempdir().unwrap();
    let module = Arc::new(module_with(Arc::new(TestEnvironment::default())));
    let kernel = kernel_with(dir.path(), Arc::clone(&module), true);
    let ctx = kernel.context();
    if !shell_or_skip(ctx) {
        return;
    }

    let saved = module
        .save_script(
            ctx,
            upsert(
                "incremental",
                ScriptType::Shell,
                "printf 'first\\n'; sleep 0.5; printf 'second\\n'\n",
            ),
        )
        .unwrap();
    let running = module.execute(ctx, saved.id, Vec::new()).unwrap();
    let mut entries = Vec::new();
    let mut since = 0;
    let deadline = Instant::now() + Duration::from_secs(6);
    while Instant::now() < deadline {
        if let Some((new_entries, record)) = module.get_logs(ctx, &running.run_id, since).unwrap() {
            for entry in new_entries {
                since = since.max(entry.seq);
                entries.push(entry);
            }
            if record.status.is_terminal() {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    wait_terminal(&module, ctx, &running.run_id);
    let combined: String = entries.iter().map(|entry| entry.text.as_str()).collect();
    assert!(combined.contains("first") || combined.contains("second"));
    assert!(entries.windows(2).all(|pair| pair[0].seq < pair[1].seq));
}

#[test]
fn invalid_inputs_and_output_modes_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let module = Arc::new(module_with(Arc::new(TestEnvironment::default())));
    let kernel = kernel_with(dir.path(), Arc::clone(&module), false);
    let ctx = kernel.context();

    assert!(module
        .save_script(ctx, upsert("", ScriptType::Shell, "echo hi\n"))
        .is_err());
    assert!(module
        .save_script(ctx, upsert("empty", ScriptType::Shell, ""))
        .is_err());
    let mut bad_timeout = upsert("bad", ScriptType::Shell, "echo hi\n");
    bad_timeout.timeout_secs = Some(0);
    assert!(module.save_script(ctx, bad_timeout).is_err());
    let mut bad_output = upsert("bad-output", ScriptType::Shell, "echo hi\n");
    bad_output.output_handling = "explode".to_string();
    assert!(module.save_script(ctx, bad_output).is_err());
    let mut bad_hotkey = upsert("bad-hotkey", ScriptType::Shell, "echo hi\n");
    bad_hotkey.hotkey = Some("NotARealKey".to_string());
    assert!(module.save_script(ctx, bad_hotkey).is_err());
    assert!(module.list_scripts(ctx).unwrap().is_empty());

    let config = ScriptRunnerConfig {
        max_log_bytes: 1,
        default_timeout_secs: 0,
        keep_run_logs: 1,
    };
    assert!(config.validate().is_err());
    assert_eq!(
        config.normalized().max_log_bytes,
        crate::config::MIN_LOG_BYTES
    );
}

#[test]
fn script_type_json_contract_matches_ui_and_reads_legacy_aliases() {
    for script_type in ScriptType::ALL {
        let json = serde_json::to_value(script_type).unwrap();
        assert_eq!(json, serde_json::json!(script_type.as_str()));
        let roundtrip: ScriptType = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip, script_type);
    }
    assert_eq!(
        serde_json::from_value::<ScriptType>(serde_json::json!("apple_script")).unwrap(),
        ScriptType::AppleScript
    );
    assert_eq!(
        serde_json::from_value::<ScriptType>(serde_json::json!("power_shell")).unwrap(),
        ScriptType::PowerShell
    );

    let legacy: ScriptUpsert = serde_json::from_value(serde_json::json!({
        "name": "legacy",
        "script_type": "apple_script",
        "is_inline": true,
        "content_or_path": "return 1\n"
    }))
    .unwrap();
    assert_eq!(legacy.script_type, ScriptType::AppleScript);
    let serialized = serde_json::to_value(&legacy).unwrap();
    assert_eq!(serialized["script_type"], "applescript");

    let legacy: ScriptUpsert = serde_json::from_value(serde_json::json!({
        "name": "legacy-ps",
        "script_type": "power_shell",
        "is_inline": true,
        "content_or_path": "Write-Output 1\n"
    }))
    .unwrap();
    assert_eq!(legacy.script_type, ScriptType::PowerShell);
    assert_eq!(
        serde_json::to_value(&legacy).unwrap()["script_type"],
        "powershell"
    );
}

#[test]
fn failing_native_hotkey_sync_rolls_back_new_script_and_core_registration() {
    let dir = tempfile::tempdir().unwrap();
    let sync = Arc::new(RecordingHotkeySync::default());
    sync.set_failing(true);
    let module = Arc::new(ScriptRunnerModule::with_services(
        Arc::new(TestEnvironment::default()),
        Arc::new(NoopScriptEventSink),
        sync.clone(),
    ));
    let kernel = kernel_with(dir.path(), Arc::clone(&module), true);
    let ctx = kernel.context();

    let mut input = upsert("sync-fail", ScriptType::Shell, "echo hi\n");
    input.hotkey = Some("PrimaryMod+Alt+F8".to_string());
    let error = module.save_script(ctx, input).unwrap_err().to_string();
    assert!(error.contains("系统全局快捷键更新失败"), "{error}");
    assert!(module.list_scripts(ctx).unwrap().is_empty());
    assert!(ctx
        .shortcut_registry
        .get_by_action(&hotkey_action_id(1))
        .is_none());
    let calls = sync.calls.lock();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].action_id, hotkey_action_id(1));
    assert_eq!(calls[0].previous, None);
    assert!(calls[0]
        .desired
        .as_deref()
        .is_some_and(|value| value.contains("F8")));
}

#[test]
fn failing_native_hotkey_sync_restores_updated_script_and_previous_shortcut() {
    let dir = tempfile::tempdir().unwrap();
    let sync = Arc::new(RecordingHotkeySync::default());
    let module = Arc::new(ScriptRunnerModule::with_services(
        Arc::new(TestEnvironment::default()),
        Arc::new(NoopScriptEventSink),
        sync.clone(),
    ));
    let kernel = kernel_with(dir.path(), Arc::clone(&module), true);
    let ctx = kernel.context();

    let mut input = upsert("original", ScriptType::Shell, "echo old\n");
    input.hotkey = Some("PrimaryMod+Alt+F7".to_string());
    let saved = module.save_script(ctx, input).unwrap();
    let old_shortcut = ctx
        .shortcut_registry
        .get_by_action(&hotkey_action_id(saved.id))
        .unwrap();

    sync.set_failing(true);
    let mut update = upsert("renamed", ScriptType::Shell, "echo new\n");
    update.id = Some(saved.id);
    update.hotkey = Some("PrimaryMod+Alt+F6".to_string());
    let error = module.save_script(ctx, update).unwrap_err().to_string();
    assert!(error.contains("脚本已回滚"), "{error}");

    let restored = module.get_script(ctx, saved.id).unwrap().unwrap();
    assert_eq!(restored.name, "original");
    assert_eq!(restored.content_or_path, "echo old\n");
    assert_eq!(restored.hotkey.as_deref(), Some("PrimaryMod+Alt+F7"));
    assert!(restored.hotkey_active);
    let restored_shortcut = ctx
        .shortcut_registry
        .get_by_action(&hotkey_action_id(saved.id))
        .unwrap();
    assert_eq!(
        restored_shortcut.physical_accelerator,
        old_shortcut.physical_accelerator
    );
}

#[test]
fn failing_native_hotkey_sync_restores_deleted_script_and_shortcut() {
    let dir = tempfile::tempdir().unwrap();
    let sync = Arc::new(RecordingHotkeySync::default());
    let module = Arc::new(ScriptRunnerModule::with_services(
        Arc::new(TestEnvironment::default()),
        Arc::new(NoopScriptEventSink),
        sync.clone(),
    ));
    let kernel = kernel_with(dir.path(), Arc::clone(&module), true);
    let ctx = kernel.context();

    let mut input = upsert("keep-on-delete-failure", ScriptType::Shell, "echo keep\n");
    input.hotkey = Some("PrimaryMod+Alt+F5".to_string());
    let saved = module.save_script(ctx, input).unwrap();
    sync.set_failing(true);
    let error = module.delete_script(ctx, saved.id).unwrap_err().to_string();
    assert!(error.contains("删除已回滚"), "{error}");

    let restored = module.get_script(ctx, saved.id).unwrap().unwrap();
    assert_eq!(restored.name, "keep-on-delete-failure");
    assert!(restored.hotkey_active);
    assert!(ctx
        .shortcut_registry
        .get_by_action(&hotkey_action_id(saved.id))
        .is_some());
}
