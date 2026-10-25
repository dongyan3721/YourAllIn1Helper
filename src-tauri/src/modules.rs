//! 具体功能的唯一装配点；注册表和 IPC 共享同一个模块实例。
use super::*;
use mod_app_usage::AppUsageModule;
// 命名装配结果避免新增功能时依赖元组顺序，降低并行开发接入出错的机会。
pub(super) struct Mounted {
    pub builder: AppKernelBuilder,
    pub screenshot: Arc<ScreenshotModule>,
    pub clipboard: Arc<ClipboardModule>,
    pub sticky_note: Arc<StickyNoteModule>,
    pub window_pin: Arc<WindowPinModule>,
    pub script_runner: Arc<ScriptRunnerModule>,
    pub screen_ink: Arc<ScreenInkModule>,
    pub media_proc: Arc<MediaProcModule>,
    pub app_usage: Arc<AppUsageModule>,
}
pub(super) fn mount(
    builder: AppKernelBuilder,
    hal: &OsHalBundle,
    script_environment: Arc<dyn ScriptEnvironment>,
    script_events: Arc<dyn ScriptEventSink>,
    script_hotkeys: Arc<dyn mod_script_runner::hotkey::ScriptHotkeySync>,
) -> CoreResult<Mounted> {
    // 截图只注入屏幕捕获与区域探测；系统剪贴板由 Tauri 生产启动流程替换注入。
    let screenshot = Arc::new(ScreenshotModule::new(
        hal.screen_capturer.clone(),
        hal.region_detector.clone(),
        Arc::new(mod_screenshot::clipboard::UnavailableClipboard),
    ));
    let clipboard = Arc::new(ClipboardModule::new(hal.input_simulator.clone()));
    let sticky_note = Arc::new(StickyNoteModule::new());
    let window_pin = Arc::new(WindowPinModule::new(hal.window_controller.clone()));
    let script_runner = Arc::new(ScriptRunnerModule::with_services(
        script_environment,
        script_events,
        script_hotkeys,
    ));
    let screen_ink = Arc::new(ScreenInkModule::new(hal.screen_capturer.clone()));
    let media_proc = Arc::new(MediaProcModule::new());
    let app_usage = Arc::new(AppUsageModule::new(hal.active_app_watcher.clone()));
    let builder = builder
        .register_module_arc(screenshot.clone())?
        .register_module_arc(clipboard.clone())?
        .register_module_arc(sticky_note.clone())?
        .register_module_arc(window_pin.clone())?
        .register_module_arc(script_runner.clone())?
        .register_module_arc(screen_ink.clone())?
        .register_module_arc(media_proc.clone())?
        .register_module_arc(app_usage.clone())?;
    Ok(Mounted {
        builder,
        screenshot,
        clipboard,
        sticky_note,
        window_pin,
        script_runner,
        screen_ink,
        media_proc,
        app_usage,
    })
}
