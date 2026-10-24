//! 具体功能的唯一装配点；注册表和 IPC 共享同一个模块实例。
use super::*;
type Mounted = (
    AppKernelBuilder,
    Arc<ClipboardModule>,
    Arc<StickyNoteModule>,
    Arc<WindowPinModule>,
    Arc<ScriptRunnerModule>,
);
pub(super) fn mount(
    builder: AppKernelBuilder,
    hal: &OsHalBundle,
    script_environment: Arc<dyn ScriptEnvironment>,
    script_events: Arc<dyn ScriptEventSink>,
    script_hotkeys: Arc<dyn mod_script_runner::hotkey::ScriptHotkeySync>,
) -> CoreResult<Mounted> {
    let clipboard = Arc::new(ClipboardModule::new(hal.input_simulator.clone()));
    let sticky = Arc::new(StickyNoteModule::new());
    let pin = Arc::new(WindowPinModule::new(hal.window_controller.clone()));
    let script_runner = Arc::new(ScriptRunnerModule::with_services(
        script_environment,
        script_events,
        script_hotkeys,
    ));
    let builder = builder
        .register_module_arc(clipboard.clone())?
        .register_module_arc(sticky.clone())?
        .register_module_arc(pin.clone())?
        .register_module_arc(script_runner.clone())?;
    Ok((builder, clipboard, sticky, pin, script_runner))
}
