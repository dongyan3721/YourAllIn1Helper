//! Explicit desktop smoke: another Win32 thread owns an isolated QA binding.
use super::*;
use crate::shortcut_conflicts::{probe_native, NativeCheck};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL, MOD_SHIFT,
};

struct ExternalBinding {
    release: std::sync::mpsc::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for ExternalBinding {
    fn drop(&mut self) {
        let _ = self.release.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
pub(super) fn check(app: &AppHandle, host: &DesktopHost) -> anyhow::Result<()> {
    let before = host.kernel().context().shortcut_registry.list_all();
    let config_path = host.kernel().context().app_data_dir.join("config.json");
    let config_before = std::fs::read(&config_path)?;
    anyhow::ensure!(
        app.global_shortcut().is_registered("F1"),
        "owned QA shortcut must exist"
    );
    anyhow::ensure!(
        probe_native(app, "F1") == NativeCheck::Available,
        "owned shortcut was classified as a conflict"
    );
    anyhow::ensure!(
        app.global_shortcut().is_registered("F1"),
        "probe removed existing shortcut"
    );
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let mut selected = None;
        for f in [23u32, 22, 24] {
            if unsafe {
                RegisterHotKey(
                    None,
                    0x6F02,
                    MOD_CONTROL | MOD_ALT | MOD_SHIFT,
                    0x70 + f - 1,
                )
            }
            .is_ok()
            {
                selected = Some(f);
                break;
            }
        }
        let _ = ready_tx.send(selected);
        if selected.is_some() {
            let _ = release_rx.recv_timeout(Duration::from_secs(30));
            let _ = unsafe { UnregisterHotKey(None, 0x6F02) };
        }
    });
    let fixture = ExternalBinding {
        release: release_tx,
        thread: Some(thread),
    };
    let f = ready_rx
        .recv_timeout(Duration::from_secs(5))?
        .ok_or_else(|| anyhow::anyhow!("No free QA fixture binding"))?;
    let accelerator = format!("Ctrl+Alt+Shift+F{f}");
    anyhow::ensure!(
        matches!(probe_native(app, &accelerator), NativeCheck::Unavailable(_)),
        "external OS binding was missed"
    );
    // Repeating the probe must not remove the other owner's binding.
    anyhow::ensure!(
        matches!(probe_native(app, &accelerator), NativeCheck::Unavailable(_)),
        "external binding was disturbed"
    );
    drop(fixture);
    anyhow::ensure!(
        probe_native(app, &accelerator) == NativeCheck::Available,
        "released key still unavailable"
    );
    anyhow::ensure!(
        !app.global_shortcut().is_registered(accelerator.as_str()),
        "temporary probe leaked a registration"
    );
    anyhow::ensure!(
        host.kernel().context().shortcut_registry.list_all() == before,
        "probe changed registry"
    );
    anyhow::ensure!(
        std::fs::read(config_path)? == config_before,
        "probe wrote configuration"
    );
    println!("UI_SMOKE_SHORTCUT_CONFLICTS: PASS (owned binding preserved, external conflict detected, temporary binding released, config unchanged)");
    Ok(())
}
