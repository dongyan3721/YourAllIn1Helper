//! Durable, rollback-aware shortcut preferences for mounted features.
use crate::*;
use app_core::{ShortcutRegistry, ShortcutSpec};

impl DesktopHost {
    fn preference_shortcut_specs(&self) -> Vec<ShortcutSpec> {
        let mut specs = self.kernel.registry().collect_shortcut_preferences();
        specs.extend(
            app_core::MODULE_CATALOG
                .iter()
                .filter(|module| self.kernel.registry().contains(module.id))
                .map(|module| {
                    ShortcutSpec::new(
                        module.id,
                        format!("{}:open_panel", module.id),
                        "",
                        format!("打开{}面板", module.name),
                    )
                }),
        );
        specs
    }

    pub(crate) fn sync_panel_shortcuts(&self) -> CoreResult<()> {
        let ctx = self.kernel.context();
        let config = ctx.config.read().clone();
        for spec in self
            .preference_shortcut_specs()
            .into_iter()
            .filter(is_panel_shortcut)
        {
            let hotkey = config
                .shortcut_overrides
                .get(&spec.action_id)
                .filter(|value| !value.trim().is_empty());
            if self.kernel.registry().is_enabled(&spec.module_id) && hotkey.is_some() {
                ctx.shortcut_registry
                    .register(spec, hotkey.map(String::as_str))?;
            } else {
                ctx.shortcut_registry.unregister(&spec.action_id);
            }
        }
        Ok(())
    }

    pub(crate) fn trigger_desktop_action(&self, action_id: &str) -> CoreResult<()> {
        if let Some(module_id) = action_id.strip_suffix(":open_panel") {
            self.open_feature_panel(module_id)?;
            return Ok(());
        }
        self.kernel.trigger_action(action_id)
    }
    pub(crate) fn set_shortcut_recorder(&self, owner: &str, recording: bool) {
        let mut active = self.shortcut_recording_owner.write();
        if recording {
            *active = Some(owner.into());
        } else if active.as_deref() == Some(owner) {
            *active = None;
        }
    }

    pub(crate) fn shortcut_preferences(&self) -> CoreResult<Value> {
        let ctx = self.kernel.context();
        let config = ctx.config.read().clone();
        let defaults = self.preference_shortcut_specs();
        let mut rows = Vec::new();
        for spec in defaults {
            let preview = ShortcutRegistry::new(ctx.shortcut_registry.target_os());
            let optional = is_panel_shortcut(&spec);
            let override_hotkey = config
                .shortcut_overrides
                .get(&spec.action_id)
                .map(String::as_str);
            let mut row = if optional && override_hotkey.is_none_or(|value| value.trim().is_empty())
            {
                json!({"spec":spec,"active_logical_hotkey":"","physical_accelerator":"","display_label":""})
            } else {
                serde_json::to_value(preview.register(spec.clone(), override_hotkey)?)?
            };
            row["optional"] = json!(optional);
            row["enabled"] = json!(self.kernel.registry().is_enabled(&spec.module_id));
            row["editable"] = json!(true);
            row["native_error"] = json!(self.native_shortcut_error(&spec.action_id));
            rows.push(row);
        }
        for binding in ctx.shortcut_registry.list_all() {
            if rows
                .iter()
                .any(|row| row["spec"]["action_id"] == binding.spec.action_id)
            {
                continue;
            }
            let mut row = serde_json::to_value(&binding)?;
            row["enabled"] = json!(true);
            row["editable"] = json!(false);
            row["native_error"] = json!(self.native_shortcut_error(&binding.spec.action_id));
            rows.push(row);
        }
        rows.sort_by(|a, b| {
            a["spec"]["action_id"]
                .as_str()
                .cmp(&b["spec"]["action_id"].as_str())
        });
        Ok(json!({"shortcuts":rows}))
    }

    pub(crate) fn update_shortcut_preference<F>(
        &self,
        action_id: &str,
        hotkey: Option<&str>,
        mut synchronize_native: F,
    ) -> CoreResult<Value>
    where
        F: FnMut() -> CoreResult<()>,
    {
        let _gate = self.shortcut_preference_gate.lock();
        let ctx = self.kernel.context();
        let specs = self.preference_shortcut_specs();
        let spec = specs
            .iter()
            .find(|s| s.action_id == action_id)
            .cloned()
            .ok_or_else(|| {
                CoreError::Config("This shortcut is managed in its feature panel".into())
            })?;
        if hotkey.is_some_and(|value| value.trim().is_empty()) {
            return Err(CoreError::Config(
                "Shortcut cannot be empty; use reset to restore its default".into(),
            ));
        }
        let before = ctx.config.read().shortcut_overrides.clone();
        let mut updated = before.clone();
        if let Some(hotkey) = hotkey {
            updated.insert(action_id.into(), hotkey.trim().into());
        } else {
            updated.remove(action_id);
        }
        // Reserve disabled feature bindings too; re-enabling must not silently change a preference.
        let preview = ShortcutRegistry::new(ctx.shortcut_registry.target_os());
        for candidate in &specs {
            let hotkey = updated.get(&candidate.action_id).map(String::as_str);
            if is_panel_shortcut(candidate) && hotkey.is_none() {
                continue;
            }
            preview.register(candidate.clone(), hotkey)?;
        }
        for dynamic in ctx.shortcut_registry.list_all() {
            if !specs.iter().any(|s| s.action_id == dynamic.spec.action_id) {
                preview.register(dynamic.spec, Some(&dynamic.active_logical_hotkey))?;
            }
        }
        let old_binding = ctx.shortcut_registry.get_by_action(action_id);
        // A failed atomic disk save must leave the runtime binding untouched.
        self.kernel
            .update_config(|config| config.shortcut_overrides = updated.clone())?;
        let apply = (|| {
            if is_panel_shortcut(&spec) && !updated.contains_key(action_id) {
                ctx.shortcut_registry.unregister(action_id);
            } else if self.kernel.registry().is_enabled(&spec.module_id) {
                ctx.shortcut_registry
                    .register(spec.clone(), updated.get(action_id).map(String::as_str))?;
            }
            synchronize_native()
        })();
        if let Err(error) = apply {
            let disk_restore = self
                .kernel
                .update_config(|config| config.shortcut_overrides = before);
            ctx.shortcut_registry.unregister(action_id);
            if let Some(binding) = old_binding {
                ctx.shortcut_registry
                    .register(binding.spec, Some(&binding.active_logical_hotkey))?;
            }
            let native_restore = synchronize_native();
            if disk_restore.is_err() || native_restore.is_err() {
                return Err(CoreError::Config(format!("Shortcut update failed ({error}); restoring the previous configuration or OS binding also failed")));
            }
            return Err(CoreError::Config(format!(
                "Shortcut update failed; previous binding restored: {error}"
            )));
        }
        self.shortcut_preferences()
    }

    pub(crate) fn open_feature_panel(&self, module_id: &str) -> CoreResult<Value> {
        let module = app_core::MODULE_CATALOG
            .iter()
            .find(|module| module.id == module_id)
            .ok_or_else(|| CoreError::Config("Unknown feature panel".into()))?;
        self.kernel.registry().with_enabled(module_id, || {
            self.kernel
                .context()
                .window_pool
                .open_window(WindowSpec::feature(module_id, module.name))?;
            Ok(json!({"label":format!("feature-{module_id}"),"module_id":module_id, "active_window_count":self.kernel.context().window_pool.active_window_count(), "active_windows":self.kernel.context().window_pool.list_active_windows()}))
        })
    }
}

fn is_panel_shortcut(spec: &ShortcutSpec) -> bool {
    spec.action_id == format!("{}:open_panel", spec.module_id)
        && spec.default_logical_hotkey.is_empty()
}

pub(crate) fn feature_for_legacy_route(route: &str) -> Option<&'static str> {
    match route
        .trim_start_matches('/')
        .trim_start_matches("dashboard/")
    {
        "screenshot" => Some("screenshot"),
        "clipboard" => Some("clipboard"),
        "screen_ink" => Some("screen_ink"),
        "sticky" => Some("sticky_note"),
        "usage" => Some("app_usage"),
        "scripts" => Some("script_runner"),
        "translator" => Some("translator"),
        "media" => Some("media_proc"),
        "winpin" => Some("window_pin"),
        "vault" => Some("password_vault"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_feature_has_an_unassigned_panel_shortcut_without_reserving_keys() {
        let dir = tempfile::tempdir().unwrap();
        let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
        let before = host.kernel.context().shortcut_registry.list_all();
        let preferences = host.shortcut_preferences().unwrap();
        let optional: Vec<_> = preferences["shortcuts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["optional"] == true)
            .collect();
        assert_eq!(optional.len(), 10);
        for row in optional {
            assert_eq!(row["active_logical_hotkey"], "");
            assert_eq!(row["physical_accelerator"], "");
            assert_eq!(row["display_label"], "");
            assert_eq!(row["enabled"], true);
        }
        assert_eq!(host.kernel.context().shortcut_registry.list_all(), before);
    }

    #[test]
    fn optional_panel_binding_survives_restart_module_cycle_and_can_be_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let action = "media_proc:open_panel";
        {
            let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
            host.update_shortcut_preference(action, Some("Ctrl+Alt+F10"), || Ok(()))
                .unwrap();
        }
        let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
        let binding = host
            .kernel
            .context()
            .shortcut_registry
            .get_by_action(action)
            .unwrap();
        assert_eq!(binding.active_logical_hotkey, "Ctrl+Alt+F10");
        let dispatched = host.invoke_ipc(
            "kernel_trigger_shortcut",
            json!({"accelerator":"Ctrl+Alt+F10"}),
        );
        assert!(dispatched.ok, "{:?}", dispatched.error);
        assert!(host
            .kernel
            .context()
            .window_pool
            .get_active_window("feature-media_proc")
            .is_some());
        let disabled = host.invoke_ipc(
            "kernel_set_module_enabled",
            json!({"module_id":"media_proc","enabled":false}),
        );
        assert!(disabled.ok, "{:?}", disabled.error);
        assert!(host
            .kernel
            .context()
            .shortcut_registry
            .get_by_action(action)
            .is_none());
        assert!(
            !host
                .invoke_ipc("kernel_trigger_shortcut", json!({"action_id":action}))
                .ok
        );
        let enabled = host.invoke_ipc(
            "kernel_set_module_enabled",
            json!({"module_id":"media_proc","enabled":true}),
        );
        assert!(enabled.ok, "{:?}", enabled.error);
        assert_eq!(
            host.kernel
                .context()
                .shortcut_registry
                .get_by_action(action)
                .unwrap(),
            binding
        );
        assert!(
            host.invoke_ipc("kernel_trigger_shortcut", json!({"action_id":action}))
                .ok
        );
        host.update_shortcut_preference(action, None, || Ok(()))
            .unwrap();
        assert!(host
            .kernel
            .context()
            .shortcut_registry
            .get_by_action(action)
            .is_none());
        let reopened = DesktopHost::bootstrap(dir.path(), true).unwrap();
        assert!(reopened
            .kernel
            .context()
            .shortcut_registry
            .get_by_action(action)
            .is_none());
    }

    #[test]
    fn optional_bindings_conflict_with_defaults_and_other_configured_panels() {
        let dir = tempfile::tempdir().unwrap();
        let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
        assert!(host
            .update_shortcut_preference("media_proc:open_panel", Some("F1"), || Ok(()))
            .is_err());
        host.update_shortcut_preference("media_proc:open_panel", Some("Ctrl+Alt+F10"), || Ok(()))
            .unwrap();
        host.invoke_ipc(
            "kernel_set_module_enabled",
            json!({"module_id":"media_proc","enabled":false}),
        );
        assert!(host
            .update_shortcut_preference("app_usage:open_panel", Some("Ctrl+Alt+F10"), || Ok(()))
            .is_err());
        let mut calls = 0;
        assert!(host
            .update_shortcut_preference("app_usage:open_panel", Some("Ctrl+Alt+F11"), || {
                calls += 1;
                if calls == 1 {
                    Err(CoreError::Config("OS conflict".into()))
                } else {
                    Ok(())
                }
            })
            .is_err());
        assert!(host
            .kernel
            .context()
            .shortcut_registry
            .get_by_action("app_usage:open_panel")
            .is_none());
        assert!(!host
            .kernel
            .context()
            .config
            .read()
            .shortcut_overrides
            .contains_key("app_usage:open_panel"));
    }

    #[test]
    fn recorder_cleanup_is_bound_to_its_native_owner() {
        let dir = tempfile::tempdir().unwrap();
        let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
        host.set_shortcut_recorder("dashboard", true);
        host.set_shortcut_recorder("feature-media_proc", true);
        host.set_shortcut_recorder("dashboard", false);
        assert_eq!(
            host.shortcut_recording_owner.read().as_deref(),
            Some("feature-media_proc")
        );
        host.set_shortcut_recorder("feature-media_proc", false);
        assert!(host.shortcut_recording_owner.read().is_none());
    }
    #[test]
    fn shortcut_updates_persist_reset_and_include_disabled_features() {
        let dir = tempfile::tempdir().unwrap();
        let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
        host.kernel.set_module_enabled("clipboard", false).unwrap();
        let result = host
            .update_shortcut_preference("clipboard:toggle_popup", Some("Ctrl+Shift+F11"), || Ok(()))
            .unwrap();
        assert!(result["shortcuts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["spec"]["action_id"] == "clipboard:toggle_popup"
                && r["enabled"] == false
                && r["active_logical_hotkey"] == "Ctrl+Shift+F11"));
        host.kernel.set_module_enabled("clipboard", true).unwrap();
        assert_eq!(
            host.kernel
                .context()
                .shortcut_registry
                .get_by_action("clipboard:toggle_popup")
                .unwrap()
                .physical_accelerator,
            "Ctrl+Shift+F11"
        );
        host.update_shortcut_preference("clipboard:toggle_popup", None, || Ok(()))
            .unwrap();
        assert!(!host
            .kernel
            .context()
            .config
            .read()
            .shortcut_overrides
            .contains_key("clipboard:toggle_popup"));
        let persisted: Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("config.json")).unwrap())
                .unwrap();
        assert!(persisted["shortcut_overrides"]
            .get("clipboard:toggle_popup")
            .is_none());
    }
    #[test]
    fn conflict_and_native_failure_preserve_previous_binding() {
        let dir = tempfile::tempdir().unwrap();
        let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
        let before = host
            .kernel
            .context()
            .shortcut_registry
            .get_by_action("clipboard:toggle_popup")
            .unwrap();
        host.kernel.set_module_enabled("screenshot", false).unwrap();
        assert!(host
            .update_shortcut_preference("clipboard:toggle_popup", Some("F1"), || Ok(()))
            .is_err());
        let mut calls = 0;
        assert!(host
            .update_shortcut_preference("clipboard:toggle_popup", Some("Ctrl+Shift+F11"), || {
                calls += 1;
                if calls == 1 {
                    Err(CoreError::Config("occupied by another app".into()))
                } else {
                    Ok(())
                }
            })
            .is_err());
        assert_eq!(calls, 2);
        assert_eq!(
            host.kernel
                .context()
                .shortcut_registry
                .get_by_action("clipboard:toggle_popup")
                .unwrap(),
            before
        );
        assert!(!host
            .kernel
            .context()
            .config
            .read()
            .shortcut_overrides
            .contains_key("clipboard:toggle_popup"));
    }
    #[test]
    fn disk_failure_does_not_change_runtime_shortcut() {
        let dir = tempfile::tempdir().unwrap();
        let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
        let before = host
            .kernel
            .context()
            .shortcut_registry
            .get_by_action("clipboard:toggle_popup")
            .unwrap();
        std::fs::remove_file(dir.path().join("config.json")).unwrap();
        std::fs::create_dir(dir.path().join("config.json")).unwrap();
        assert!(host
            .update_shortcut_preference(
                "clipboard:toggle_popup",
                Some("Ctrl+Shift+F11"),
                || panic!("native must not be called")
            )
            .is_err());
        assert_eq!(
            host.kernel
                .context()
                .shortcut_registry
                .get_by_action("clipboard:toggle_popup")
                .unwrap(),
            before
        );
    }
    #[test]
    fn feature_panels_reuse_labels_reject_unknown_and_close_on_disable() {
        let dir = tempfile::tempdir().unwrap();
        let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
        host.open_feature_panel("media_proc").unwrap();
        host.open_feature_panel("media_proc").unwrap();
        assert_eq!(host.kernel.context().window_pool.active_window_count(), 1);
        assert!(host.open_feature_panel("../other").is_err());
        host.invoke_ipc(
            "kernel_set_module_enabled",
            json!({"module_id":"media_proc","enabled":false}),
        );
        assert_eq!(host.kernel.context().window_pool.active_window_count(), 0);
        assert!(host.open_feature_panel("media_proc").is_err());
    }
}
