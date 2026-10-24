//! 具体功能的唯一装配点；注册表和 IPC 共享同一个模块实例。
use super::*;
type Mounted = (
    AppKernelBuilder,
    Arc<ClipboardModule>,
    Arc<MediaProcModule>,
    Arc<StickyNoteModule>,
    Arc<WindowPinModule>,
);
pub(super) fn mount(builder: AppKernelBuilder, hal: &OsHalBundle) -> CoreResult<Mounted> {
    let clipboard = Arc::new(ClipboardModule::new(hal.input_simulator.clone()));
    let media = Arc::new(MediaProcModule::new());
    let sticky = Arc::new(StickyNoteModule::new());
    let pin = Arc::new(WindowPinModule::new(hal.window_controller.clone()));
    let builder = builder
        .register_module_arc(clipboard.clone())?
        .register_module_arc(media.clone())?
        .register_module_arc(sticky.clone())?
        .register_module_arc(pin.clone())?;
    Ok((builder, clipboard, media, sticky, pin))
}
