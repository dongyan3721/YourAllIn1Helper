//! Cross-platform global shortcut normalization and conflict-aware registry.
//!
//! Normalizes platform-independent logical modifier expressions (`PrimaryMod`,
//! `SecondaryMod`, `MetaMod`) into Windows and macOS physical key combinations.

use std::collections::HashMap;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use crate::error::{CoreError, CoreResult};

/// Target operating system used for hotkey normalization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetOs {
    Windows,
    MacOs,
    Linux,
}

impl TargetOs {
    /// Returns the compile-time target OS.
    pub const fn current() -> Self {
        #[cfg(target_os = "windows")]
        {
            Self::Windows
        }
        #[cfg(target_os = "macos")]
        {
            Self::MacOs
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            Self::Linux
        }
    }
}

/// Specification for a module shortcut action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShortcutSpec {
    /// Unique action identifier (e.g., `"screenshot:capture"`, `"clipboard:toggle_popup"`).
    pub action_id: String,
    /// Owning module ID (e.g., `"screenshot"`, `"clipboard"`).
    pub module_id: String,
    /// Platform-neutral logical hotkey expression (e.g., `"PrimaryMod+Shift+V"`, `"F1"`).
    pub default_logical_hotkey: String,
    /// Human-readable description of the shortcut action.
    pub description: String,
}

impl ShortcutSpec {
    pub fn new(
        module_id: impl Into<String>,
        action_id: impl Into<String>,
        default_logical_hotkey: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            action_id: action_id.into(),
            module_id: module_id.into(),
            default_logical_hotkey: default_logical_hotkey.into(),
            description: description.into(),
        }
    }
}

/// Active binding record stored in `ShortcutRegistry`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisteredShortcut {
    pub spec: ShortcutSpec,
    /// Effective logical hotkey (either user override or module default).
    pub active_logical_hotkey: String,
    /// Normalized physical accelerator string for `tauri-plugin-global-shortcut`.
    pub physical_accelerator: String,
    /// UI-friendly display string (e.g. `⌘ + ⇧ + V` on macOS, `Ctrl + Shift + V` on Windows).
    pub display_label: String,
}

/// Stateless utility that converts logical shortcut strings into OS-specific physical accelerators.
pub struct ShortcutNormalizer;

impl ShortcutNormalizer {
    /// Normalizes a logical hotkey expression (such as `"PrimaryMod+Shift+V"` or `"SecondaryMod+F1"`)
    /// into a canonical physical accelerator string for the specified `TargetOs`.
    ///
    /// Mapping rules:
    /// - `PrimaryMod` -> `Ctrl` on Windows/Linux, `Super` (`Command`) on macOS
    /// - `SecondaryMod` -> `Alt` on Windows/Linux, `Alt` (`Option`) on macOS
    /// - `MetaMod` -> `Super` (`Win`) on Windows/Linux, `Ctrl` (`Control`) on macOS
    pub fn normalize_for_os(logical_hotkey: &str, os: TargetOs) -> CoreResult<String> {
        let trimmed = logical_hotkey.trim();
        if trimmed.is_empty() {
            return Err(CoreError::InvalidShortcut {
                hotkey: logical_hotkey.to_string(),
                reason: "Shortcut expression cannot be empty".to_string(),
            });
        }

        let mut modifiers: Vec<&'static str> = Vec::new();
        let mut main_key: Option<String> = None;

        for raw_token in trimmed.split('+').map(str::trim) {
            if raw_token.is_empty() {
                return Err(CoreError::InvalidShortcut {
                    hotkey: logical_hotkey.to_string(),
                    reason: "Empty token around '+' delimiter".to_string(),
                });
            }

            let upper = raw_token.to_ascii_uppercase();
            match upper.as_str() {
                "PRIMARYMOD" | "CMDORCTRL" | "COMMANDORCONTROL" => {
                    let mapped = match os {
                        TargetOs::MacOs => "Super",
                        TargetOs::Windows | TargetOs::Linux => "Ctrl",
                    };
                    if !modifiers.contains(&mapped) {
                        modifiers.push(mapped);
                    }
                }
                "SECONDARYMOD" => {
                    let mapped = "Alt";
                    if !modifiers.contains(&mapped) {
                        modifiers.push(mapped);
                    }
                }
                "METAMOD" => {
                    let mapped = match os {
                        TargetOs::MacOs => "Ctrl",
                        TargetOs::Windows | TargetOs::Linux => "Super",
                    };
                    if !modifiers.contains(&mapped) {
                        modifiers.push(mapped);
                    }
                }
                "CTRL" | "CONTROL" => {
                    if !modifiers.contains(&"Ctrl") {
                        modifiers.push("Ctrl");
                    }
                }
                "ALT" | "OPTION" => {
                    if !modifiers.contains(&"Alt") {
                        modifiers.push("Alt");
                    }
                }
                "SHIFT" => {
                    if !modifiers.contains(&"Shift") {
                        modifiers.push("Shift");
                    }
                }
                "SUPER" | "CMD" | "COMMAND" | "WIN" | "META" => {
                    if !modifiers.contains(&"Super") {
                        modifiers.push("Super");
                    }
                }
                _ => {
                    if main_key.is_some() {
                        return Err(CoreError::InvalidShortcut {
                            hotkey: logical_hotkey.to_string(),
                            reason: format!(
                                "Multiple non-modifier keys found ('{}')",
                                raw_token
                            ),
                        });
                    }
                    main_key = Some(Self::canonicalize_main_key(&upper)?);
                }
            }
        }

        let key = main_key.ok_or_else(|| CoreError::InvalidShortcut {
            hotkey: logical_hotkey.to_string(),
            reason: "Shortcut must contain a non-modifier trigger key (e.g. 'V', 'F1', 'Space')"
                .to_string(),
        })?;

        // Enforce deterministic modifier ordering: Ctrl -> Alt -> Shift -> Super
        modifiers.sort_by_key(|m| match *m {
            "Ctrl" => 0,
            "Alt" => 1,
            "Shift" => 2,
            "Super" => 3,
            _ => 4,
        });

        let mut parts: Vec<String> = modifiers.into_iter().map(str::to_string).collect();
        parts.push(key);
        Ok(parts.join("+"))
    }

    /// Formats a physical or logical hotkey into a user-friendly UI badge string.
    pub fn format_display_label(logical_hotkey: &str, os: TargetOs) -> CoreResult<String> {
        let physical = Self::normalize_for_os(logical_hotkey, os)?;
        let formatted: Vec<&str> = physical
            .split('+')
            .map(|token| match (os, token) {
                (TargetOs::MacOs, "Super") => "⌘",
                (TargetOs::MacOs, "Shift") => "⇧",
                (TargetOs::MacOs, "Alt") => "⌥",
                (TargetOs::MacOs, "Ctrl") => "⌃",
                (_, "Super") => "Win",
                (_, other) => other,
            })
            .collect();
        Ok(formatted.join(" + "))
    }

    fn canonicalize_main_key(upper_key: &str) -> CoreResult<String> {
        if upper_key.len() == 1 {
            let ch = upper_key.chars().next().unwrap();
            if ch.is_ascii_alphanumeric() {
                return Ok(upper_key.to_string());
            }
        }

        // Function keys F1..F24
        if let Some(num_str) = upper_key.strip_prefix('F') {
            if let Ok(num) = num_str.parse::<u8>() {
                if (1..=24).contains(&num) {
                    return Ok(format!("F{}", num));
                }
            }
        }

        match upper_key {
            "SPACE" | "TAB" | "ENTER" | "RETURN" | "ESCAPE" | "ESC" | "BACKSPACE" | "DELETE"
            | "INSERT" | "HOME" | "END" | "PAGEUP" | "PAGEDOWN" | "UP" | "DOWN" | "LEFT"
            | "RIGHT" | "GRAVE" | "BACKQUOTE" | "MINUS" | "EQUAL" => {
                let normalized = match upper_key {
                    "ESC" => "Escape",
                    "RETURN" => "Enter",
                    "BACKQUOTE" => "Grave",
                    other => {
                        return Ok(format!(
                            "{}{}",
                            &other[0..1],
                            other[1..].to_ascii_lowercase()
                        ))
                    }
                };
                Ok(normalized.to_string())
            }
            _ => Err(CoreError::InvalidShortcut {
                hotkey: upper_key.to_string(),
                reason: format!("Unsupported key token '{}'", upper_key),
            }),
        }
    }
}

/// Thread-safe registry managing all module and dynamic script shortcuts.
pub struct ShortcutRegistry {
    target_os: TargetOs,
    /// Map from `action_id` -> `RegisteredShortcut`
    by_action: RwLock<HashMap<String, RegisteredShortcut>>,
    /// Map from `physical_accelerator` -> `action_id`
    by_accelerator: RwLock<HashMap<String, String>>,
}

impl Default for ShortcutRegistry {
    fn default() -> Self {
        Self::new(TargetOs::current())
    }
}

impl ShortcutRegistry {
    pub fn new(target_os: TargetOs) -> Self {
        Self {
            target_os,
            by_action: RwLock::new(HashMap::new()),
            by_accelerator: RwLock::new(HashMap::new()),
        }
    }

    /// Registers a shortcut specification with an optional user override hotkey.
    /// Fails with `CoreError::ShortcutConflict` if another action already claims the same physical accelerator.
    pub fn register(
        &self,
        spec: ShortcutSpec,
        user_override_hotkey: Option<&str>,
    ) -> CoreResult<RegisteredShortcut> {
        let effective_logical = user_override_hotkey
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&spec.default_logical_hotkey)
            .to_string();

        let physical = ShortcutNormalizer::normalize_for_os(&effective_logical, self.target_os)?;
        let display_label =
            ShortcutNormalizer::format_display_label(&effective_logical, self.target_os)?;

        let mut by_action = self.by_action.write();
        let mut by_accel = self.by_accelerator.write();

        if let Some(existing_action_id) = by_accel.get(&physical) {
            if existing_action_id != &spec.action_id {
                let existing_module = by_action
                    .get(existing_action_id)
                    .map(|r| r.spec.module_id.clone())
                    .unwrap_or_else(|| "unknown".to_string());
                return Err(CoreError::ShortcutConflict {
                    hotkey: physical,
                    new_action: spec.action_id,
                    existing_action: existing_action_id.clone(),
                    existing_module,
                });
            }
        }

        // Remove previous accelerator mapping if updating an existing action
        if let Some(prev) = by_action.get(&spec.action_id) {
            by_accel.remove(&prev.physical_accelerator);
        }

        let record = RegisteredShortcut {
            spec: spec.clone(),
            active_logical_hotkey: effective_logical,
            physical_accelerator: physical.clone(),
            display_label,
        };

        by_accel.insert(physical, spec.action_id.clone());
        by_action.insert(spec.action_id, record.clone());
        Ok(record)
    }

    /// Unregisters a shortcut by `action_id` (e.g., when a custom script is deleted).
    pub fn unregister(&self, action_id: &str) -> Option<RegisteredShortcut> {
        let mut by_action = self.by_action.write();
        let mut by_accel = self.by_accelerator.write();
        if let Some(removed) = by_action.remove(action_id) {
            by_accel.remove(&removed.physical_accelerator);
            Some(removed)
        } else {
            None
        }
    }

    /// Resolves a physical accelerator (or logical expression) to its `(module_id, action_id)`.
    pub fn resolve_accelerator(&self, accelerator: &str) -> Option<(String, String)> {
        let normalized = ShortcutNormalizer::normalize_for_os(accelerator, self.target_os)
            .unwrap_or_else(|_| accelerator.to_string());
        let by_accel = self.by_accelerator.read();
        let by_action = self.by_action.read();
        let action_id = by_accel.get(&normalized)?;
        let record = by_action.get(action_id)?;
        Some((record.spec.module_id.clone(), record.spec.action_id.clone()))
    }

    /// Resolves an `action_id` directly to its `RegisteredShortcut`.
    pub fn get_by_action(&self, action_id: &str) -> Option<RegisteredShortcut> {
        self.by_action.read().get(action_id).cloned()
    }

    /// Lists all registered shortcuts sorted by `action_id`.
    pub fn list_all(&self) -> Vec<RegisteredShortcut> {
        let mut list: Vec<RegisteredShortcut> =
            self.by_action.read().values().cloned().collect();
        list.sort_by(|a, b| a.spec.action_id.cmp(&b.spec.action_id));
        list
    }
}
