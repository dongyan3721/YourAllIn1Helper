//! `all-in-one-host` (`src-tauri`) — Microkernel Desktop Host & IPC Bridge.
//!
//! Assembles the `AppKernel` daemon and mounts the implemented feature modules:
//! 1. **Module 02 (`mod-clipboard`)**: Ditto-style SHA-256 deduplicated LRU-100 clipboard manager.
//! 2. **Module 04 (`mod-sticky-note`)**: Pinned desktop Markdown + LaTeX rich-text sticky notes.
//! 3. **Module 06 (`mod-script-runner`)**: Cross-platform script library with isolated execution.
//! 4. **Module 09 (`mod-window-pin`)**: DeskPins-style external window topmost controller.
//!
//! Exposes a unified JSON IPC command dispatcher (`DesktopHost::invoke_ipc`). Production uses
//! the real Tauri 2.x shell in `tauri_app.rs`; no Swift host or local HTTP server is started.

use app_core::{
    AppKernel, AppKernelBuilder, CoreError, CoreResult, ExternalToolKind, HeadlessWindowBackend,
    TargetOs, WindowBackend, WindowSpec,
};
use mod_app_usage::AppUsageModule;
use mod_clipboard::ClipboardModule;
use mod_media_proc::MediaProcModule;
use mod_password_vault::PasswordVaultModule;
use mod_screen_ink::ScreenInkModule;
use mod_screenshot::ScreenshotModule;
use mod_script_runner::env::ScriptEnvironment;
use mod_script_runner::events::ScriptEventSink;
use mod_script_runner::hotkey::ScriptHotkeySync;
use mod_script_runner::ScriptRunnerModule;
use mod_sticky_note::{StickyNoteModule, StickyNoteWindowStateUpdate};
use mod_translator::TranslatorModule;
use mod_window_pin::WindowPinModule;
use os_hal::{ExternalWindowInfo, MockOsHal, NativeOsHal, OsHalBundle};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

mod clipboard_window;
mod demo;
mod exit_coordinator;
mod ink_window;
mod ipc;
mod legacy_http;
mod modules;
mod native_dialog;
mod screenshot_binary;
mod screenshot_clipboard;
mod screenshot_window;
mod script_env;
mod script_events;
mod script_hotkeys;
mod secure_credentials;
mod sticky_flush;
mod tauri_app;
mod translator_mouse;
pub use legacy_http::serve_desktop_ui;
pub use tauri_app::run;

/// Request payload sent to `POST /api/ipc` or `DesktopHost::invoke_ipc`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcRequest {
    pub cmd: String,
    #[serde(default)]
    pub args: Value,
}

/// Standardized IPC response envelope returned to `src-ui`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcResponse {
    pub ok: bool,
    pub data: Option<Value>,
    pub error: Option<String>,
}

impl IpcResponse {
    pub fn success(data: impl Serialize) -> Self {
        Self {
            ok: true,
            data: serde_json::to_value(data).ok(),
            error: None,
        }
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(msg.into()),
        }
    }
}

/// Assembled desktop host holding the running `AppKernel` and mounted feature module handles.
pub struct DesktopHost {
    kernel: AppKernel,
    screenshot: Arc<ScreenshotModule>,
    clipboard: Arc<ClipboardModule>,
    sticky_note: Arc<StickyNoteModule>,
    window_pin: Arc<WindowPinModule>,
    script_runner: Arc<ScriptRunnerModule>,
    screen_ink: Arc<ScreenInkModule>,
    media_proc: Arc<MediaProcModule>,
    app_usage: Arc<AppUsageModule>,
    translator: Arc<TranslatorModule>,
    password_vault: Arc<PasswordVaultModule>,
    translator_credentials: Arc<secure_credentials::HostCredentialStore>,
    os_hal: OsHalBundle,
    mock_hal: Option<MockOsHal>,
    /// 系统全局快捷键注册失败记录（action_id → 错误），供界面如实显示未生效的快捷键。
    native_shortcut_failures: parking_lot::RwLock<HashMap<String, String>>,
}

impl DesktopHost {
    /// Bootstraps the `AppKernel` and mounts Modules 02 (`mod-clipboard`), 04 (`mod-sticky-note`),
    /// and 09 (`mod-window-pin`).
    pub fn bootstrap(app_data_dir: impl Into<PathBuf>, in_memory_db: bool) -> CoreResult<Self> {
        let mock = MockOsHal::new();
        Self::assemble(
            app_data_dir,
            in_memory_db,
            Arc::new(HeadlessWindowBackend),
            mock.clone().into_bundle(),
            Some(mock),
            Arc::new(mod_script_runner::UnavailableScriptEnvironment),
            Arc::new(mod_script_runner::events::NoopScriptEventSink),
            Arc::new(mod_script_runner::hotkey::NoopScriptHotkeySync),
        )
    }

    /// Bootstraps with native OS capabilities and a real window backend (Tauri in production).
    pub fn bootstrap_with_window_backend(
        app_data_dir: impl Into<PathBuf>,
        in_memory_db: bool,
        window_backend: Arc<dyn WindowBackend>,
    ) -> CoreResult<Self> {
        Self::bootstrap_with_adapters(
            app_data_dir,
            in_memory_db,
            window_backend,
            NativeOsHal::new().into_bundle(),
        )
    }

    /// Composition root for alternative adapters; features never depend on `NativeOsHal`.
    pub fn bootstrap_with_adapters(
        app_data_dir: impl Into<PathBuf>,
        in_memory_db: bool,
        window_backend: Arc<dyn WindowBackend>,
        os_hal: OsHalBundle,
    ) -> CoreResult<Self> {
        Self::bootstrap_with_adapters_and_scripting(
            app_data_dir,
            in_memory_db,
            window_backend,
            os_hal,
            Arc::new(mod_script_runner::UnavailableScriptEnvironment),
            Arc::new(mod_script_runner::events::NoopScriptEventSink),
        )
    }

    /// 兼容入口：无系统快捷键同步器时仅维护核心注册表（自动化/无桌面测试）。
    pub fn bootstrap_with_adapters_and_scripting(
        app_data_dir: impl Into<PathBuf>,
        in_memory_db: bool,
        window_backend: Arc<dyn WindowBackend>,
        os_hal: OsHalBundle,
        script_environment: Arc<dyn ScriptEnvironment>,
        script_events: Arc<dyn ScriptEventSink>,
    ) -> CoreResult<Self> {
        Self::bootstrap_with_adapters_and_services(
            app_data_dir,
            in_memory_db,
            window_backend,
            os_hal,
            script_environment,
            script_events,
            Arc::new(mod_script_runner::hotkey::NoopScriptHotkeySync),
        )
    }

    /// 完整生产入口：显式注入事件桥与原生全局快捷键同步器。
    pub fn bootstrap_with_adapters_and_services(
        app_data_dir: impl Into<PathBuf>,
        in_memory_db: bool,
        window_backend: Arc<dyn WindowBackend>,
        os_hal: OsHalBundle,
        script_environment: Arc<dyn ScriptEnvironment>,
        script_events: Arc<dyn ScriptEventSink>,
        script_hotkeys: Arc<dyn ScriptHotkeySync>,
    ) -> CoreResult<Self> {
        Self::assemble(
            app_data_dir,
            in_memory_db,
            window_backend,
            os_hal,
            None,
            script_environment,
            script_events,
            script_hotkeys,
        )
    }

    /// 便捷入口：使用原生 OS HAL 与显式脚本环境。
    pub fn bootstrap_with_window_backend_and_scripting(
        app_data_dir: impl Into<PathBuf>,
        in_memory_db: bool,
        window_backend: Arc<dyn WindowBackend>,
        script_environment: Arc<dyn ScriptEnvironment>,
        script_events: Arc<dyn ScriptEventSink>,
    ) -> CoreResult<Self> {
        Self::bootstrap_with_adapters_and_scripting(
            app_data_dir,
            in_memory_db,
            window_backend,
            NativeOsHal::new().into_bundle(),
            script_environment,
            script_events,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn assemble(
        app_data_dir: impl Into<PathBuf>,
        in_memory_db: bool,
        window_backend: Arc<dyn WindowBackend>,
        os_hal: OsHalBundle,
        mock_hal: Option<MockOsHal>,
        script_environment: Arc<dyn ScriptEnvironment>,
        script_events: Arc<dyn ScriptEventSink>,
        script_hotkeys: Arc<dyn ScriptHotkeySync>,
    ) -> CoreResult<Self> {
        let builder = AppKernelBuilder::new(app_data_dir)
            .with_in_memory_db(in_memory_db)
            .with_target_os(TargetOs::current())
            .with_window_backend(window_backend);
        let modules::Mounted {
            builder,
            screenshot,
            clipboard,
            sticky_note,
            window_pin,
            script_runner,
            screen_ink,
            media_proc,
            app_usage,
            translator,
            password_vault,
            translator_credentials,
        } = modules::mount(
            builder,
            &os_hal,
            script_environment,
            script_events,
            script_hotkeys,
        )?;
        Ok(Self {
            kernel: builder.bootstrap()?,
            screenshot,
            clipboard,
            sticky_note,
            window_pin,
            script_runner,
            screen_ink,
            media_proc,
            app_usage,
            translator,
            password_vault,
            translator_credentials,
            os_hal,
            mock_hal,
            native_shortcut_failures: parking_lot::RwLock::new(HashMap::new()),
        })
    }

    /// Returns a reference to the underlying `AppKernel`.
    pub fn kernel(&self) -> &AppKernel {
        &self.kernel
    }

    /// 增强截图模块实例（与注册表中挂载的是同一个 Arc）。
    pub fn screenshot(&self) -> &Arc<ScreenshotModule> {
        &self.screenshot
    }

    /// 用本轮系统注册结果替换失败记录。
    pub(crate) fn set_native_shortcut_failures(&self, failures: HashMap<String, String>) {
        *self.native_shortcut_failures.write() = failures;
    }

    /// 某个动作的系统快捷键注册错误；`None` 表示未失败（或尚未进行系统注册）。
    pub fn native_shortcut_error(&self, action_id: &str) -> Option<String> {
        self.native_shortcut_failures.read().get(action_id).cloned()
    }

    /// Dispatches a typed IPC command from the frontend (`src-ui`) or integration tests.
    pub fn invoke_ipc(&self, cmd: &str, args: Value) -> IpcResponse {
        match self.dispatch_inner(cmd, args) {
            Ok(val) => IpcResponse::success(val),
            Err(err) => IpcResponse::err(err.to_string()),
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/lib.rs"]
mod tests;
