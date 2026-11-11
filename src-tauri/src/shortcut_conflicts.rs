//! Non-destructive shortcut validation and short-lived native registration probes.
use crate::*;
use app_core::ShortcutNormalizer;
use tauri::AppHandle;
use tauri_plugin_global_shortcut::GlobalShortcutExt;

// Serialize actual OS registration changes, including dynamic script bindings.
pub(crate) static NATIVE_SHORTCUT_GATE: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

#[derive(Debug, PartialEq)]
pub(crate) enum NativeCheck {
    Available,
    Unavailable(String),
    ProbeError(String),
    NotChecked,
}

trait ProbeBackend {
    fn owns(&self, accelerator: &str) -> bool;
    fn register(&self, accelerator: &str) -> Result<(), String>;
    fn unregister(&self, accelerator: &str) -> Result<(), String>;
}

fn probe(backend: &impl ProbeBackend, accelerator: &str) -> NativeCheck {
    // Never unregister or replace an existing application callback to test it.
    if backend.owns(accelerator) {
        return NativeCheck::Available;
    }
    if let Err(error) = backend.register(accelerator) {
        return NativeCheck::Unavailable(error);
    }
    if let Err(error) = backend.unregister(accelerator) {
        // One cleanup retry; never claim availability if cleanup still fails.
        if let Err(retry) = backend.unregister(accelerator) {
            return NativeCheck::ProbeError(format!(
                "Temporary shortcut cleanup failed: {error}; {retry}"
            ));
        }
    }
    NativeCheck::Available
}

struct NativeBackend<'a>(&'a AppHandle);
impl ProbeBackend for NativeBackend<'_> {
    fn owns(&self, accelerator: &str) -> bool {
        self.0.global_shortcut().is_registered(accelerator)
    }
    fn register(&self, accelerator: &str) -> Result<(), String> {
        // No callback: testing never invokes a feature or a script.
        self.0
            .global_shortcut()
            .register(accelerator)
            .map_err(|e| e.to_string())
    }
    fn unregister(&self, accelerator: &str) -> Result<(), String> {
        self.0
            .global_shortcut()
            .unregister(accelerator)
            .map_err(|e| e.to_string())
    }
}
pub(crate) fn probe_native(app: &AppHandle, accelerator: &str) -> NativeCheck {
    let _guard = NATIVE_SHORTCUT_GATE.lock();
    if crate::tauri_app::is_exiting(app) {
        return NativeCheck::ProbeError("Application is exiting".into());
    }
    probe(&NativeBackend(app), accelerator)
}

impl DesktopHost {
    pub(crate) fn check_shortcut<F>(
        &self,
        action: &str,
        hotkey: Option<&str>,
        mut native: F,
    ) -> CoreResult<Value>
    where
        F: FnMut(&str) -> NativeCheck,
    {
        let preferences = self.shortcut_preferences()?;
        let rows = preferences["shortcuts"].as_array().unwrap();
        let row = rows
            .iter()
            .find(|row| row["spec"]["action_id"] == action)
            .ok_or_else(|| CoreError::Config("Unknown shortcut action".into()))?;
        let effective =
            hotkey.unwrap_or_else(|| row["spec"]["default_logical_hotkey"].as_str().unwrap_or(""));
        let mut check = json!({"action_id":action,"hotkey":effective,"display_label":"","status":"unassigned","conflicts":[]});
        if effective.is_empty() && row["optional"] == true {
            return Ok(check);
        }
        let os = self.kernel.context().shortcut_registry.target_os();
        let physical = match ShortcutNormalizer::normalize_for_os(effective, os) {
            Ok(value) => value,
            Err(error) => {
                check["status"] = json!("invalid");
                check["detail"] = json!(error.to_string());
                return Ok(check);
            }
        };
        check["physical_accelerator"] = json!(physical);
        check["display_label"] = json!(ShortcutNormalizer::format_display_label(effective, os)?);
        let conflicts: Vec<_> = rows
            .iter()
            .filter(|other| {
                other["spec"]["action_id"] != action
                    && other["physical_accelerator"].as_str() == Some(&physical)
            })
            .cloned()
            .collect();
        if !conflicts.is_empty() {
            check["status"] = json!("internal_conflict");
            check["conflicts"] = json!(conflicts);
            return Ok(check);
        }
        let (status, detail) = match native(&physical) {
            NativeCheck::Available => ("available", None),
            NativeCheck::Unavailable(error) => ("os_unavailable", Some(error)),
            NativeCheck::ProbeError(error) => ("probe_error", Some(error)),
            NativeCheck::NotChecked => ("not_checked", None),
        };
        check["status"] = json!(status);
        check["detail"] = json!(detail);
        Ok(check)
    }

    pub(crate) fn check_all_shortcuts<F>(&self, mut native: F) -> CoreResult<Value>
    where
        F: FnMut(&str) -> NativeCheck,
    {
        let preferences = self.shortcut_preferences()?;
        let mut checks = vec![];
        for row in preferences["shortcuts"].as_array().unwrap() {
            let action = row["spec"]["action_id"].as_str().unwrap();
            let hotkey = row["active_logical_hotkey"].as_str().unwrap_or("");
            checks.push(self.check_shortcut(action, Some(hotkey), &mut native)?);
        }
        Ok(json!({"checks":checks}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_core::ShortcutSpec;
    use std::cell::Cell;
    struct Fake {
        owned: bool,
        occupied: bool,
        cleanup_fails: bool,
        registers: Cell<u32>,
        unregisters: Cell<u32>,
    }
    impl ProbeBackend for Fake {
        fn owns(&self, _: &str) -> bool {
            self.owned
        }
        fn register(&self, _: &str) -> Result<(), String> {
            self.registers.set(self.registers.get() + 1);
            if self.occupied {
                Err("occupied".into())
            } else {
                Ok(())
            }
        }
        fn unregister(&self, _: &str) -> Result<(), String> {
            self.unregisters.set(self.unregisters.get() + 1);
            if self.cleanup_fails {
                Err("cleanup failed".into())
            } else {
                Ok(())
            }
        }
    }
    fn fake(owned: bool, occupied: bool, cleanup_fails: bool) -> Fake {
        Fake {
            owned,
            occupied,
            cleanup_fails,
            registers: Cell::new(0),
            unregisters: Cell::new(0),
        }
    }
    #[test]
    fn native_probe_preserves_owned_callbacks_and_never_unregisters_external_bindings() {
        let owned = fake(true, false, false);
        assert_eq!(probe(&owned, "F1"), NativeCheck::Available);
        assert_eq!((owned.registers.get(), owned.unregisters.get()), (0, 0));
        let external = fake(false, true, false);
        assert!(matches!(
            probe(&external, "F1"),
            NativeCheck::Unavailable(_)
        ));
        assert_eq!(external.unregisters.get(), 0);
        let available = fake(false, false, false);
        assert_eq!(probe(&available, "F1"), NativeCheck::Available);
        assert_eq!(
            (available.registers.get(), available.unregisters.get()),
            (1, 1)
        );
        assert!(matches!(
            probe(&fake(false, false, true), "F1"),
            NativeCheck::ProbeError(_)
        ));
    }
    #[test]
    fn checks_find_normalized_disabled_and_script_conflicts_without_mutating_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
        let before = std::fs::read(dir.path().join("config.json")).unwrap();
        host.kernel.set_module_enabled("screenshot", false).unwrap();
        let before = std::fs::read(dir.path().join("config.json")).unwrap_or(before);
        let check = host
            .check_shortcut(
                "clipboard:toggle_popup",
                Some("PrimaryMod+SecondaryMod+A"),
                |_| panic!("internal conflict must not probe OS"),
            )
            .unwrap();
        assert_eq!(check["status"], "internal_conflict");
        assert_eq!(
            check["conflicts"][0]["spec"]["action_id"],
            "screenshot:capture"
        );
        host.kernel
            .context()
            .shortcut_registry
            .register(
                ShortcutSpec::new("script_runner", "script:qa", "Ctrl+Shift+F12", "QA script"),
                None,
            )
            .unwrap();
        assert_eq!(
            host.check_shortcut(
                "media_proc:open_panel",
                Some("Shift+Ctrl+F12"),
                |_| panic!()
            )
            .unwrap()["status"],
            "internal_conflict"
        );
        assert_eq!(
            host.check_shortcut("media_proc:open_panel", None, |_| panic!())
                .unwrap()["status"],
            "unassigned"
        );
        assert_eq!(
            host.check_shortcut(
                "media_proc:open_panel",
                Some("bad+expression"),
                |_| panic!()
            )
            .unwrap()["status"],
            "invalid"
        );
        assert_eq!(
            std::fs::read(dir.path().join("config.json")).unwrap(),
            before
        );
    }
    #[test]
    fn scans_report_os_unavailability_and_headless_unknown_without_probing_unassigned_keys() {
        let dir = tempfile::tempdir().unwrap();
        let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
        assert_eq!(
            host.check_shortcut("clipboard:toggle_popup", Some("Ctrl+Shift+V"), |_| {
                NativeCheck::Unavailable("reserved".into())
            })
            .unwrap()["status"],
            "os_unavailable"
        );
        let mut count = 0;
        let scan = host
            .check_all_shortcuts(|_| {
                count += 1;
                NativeCheck::NotChecked
            })
            .unwrap();
        assert_eq!(count, 8);
        assert_eq!(scan["checks"].as_array().unwrap().len(), 21);
        assert_eq!(
            scan["checks"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|v| v["status"] == "unassigned")
                .count(),
            13
        );
    }
}
