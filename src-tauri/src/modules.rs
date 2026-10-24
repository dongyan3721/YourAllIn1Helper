//! 具体功能的唯一装配点；注册表和 IPC 共享同一个模块实例。
use super::*;
type Mounted = (
    AppKernelBuilder,
    Arc<ClipboardModule>,
    Arc<StickyNoteModule>,
    Arc<WindowPinModule>,
    Arc<ScreenInkModule>,
);
pub(super) fn mount(builder: AppKernelBuilder, hal: &OsHalBundle) -> CoreResult<Mounted> {
    let clipboard = Arc::new(ClipboardModule::new(hal.input_simulator.clone()));
    let sticky = Arc::new(StickyNoteModule::new());
    let pin = Arc::new(WindowPinModule::new(hal.window_controller.clone()));
    let ink = Arc::new(ScreenInkModule::new(hal.screen_capturer.clone()));
    let builder = builder
        .register_module_arc(clipboard.clone())?
        .register_module_arc(sticky.clone())?
        .register_module_arc(pin.clone())?
        .register_module_arc(ink.clone())?;
    Ok((builder, clipboard, sticky, pin, ink))
}
