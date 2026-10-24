//! Three-level WebView lifecycle manager (`WindowPoolManager`).
//!
//! Enforces the <25MB background memory target by ensuring:
//! - **Level 0 (Daemon Only)**: Zero WebView windows created on startup.
//! - **Level 1 (Transient Window)**: Created on demand via `WebviewWindowBuilder`,
//!   and strictly **destroyed** (`window.destroy()`) upon close or blur rather than hidden.
//! - **Level 2 (Pinned Resident Window)**: Kept alive only while actively pinned on desktop
//!   (e.g., Snipaste pinned image `pin-img-{id}` or `sticky-note-{id}`), and immediately
//!   destroyed when closed.

use crate::error::{CoreError, CoreResult};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use tracing::info;

/// Memory-saving lifecycle tier for a WebView window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowLifecyclePolicy {
    /// Transient window: destroyed (`destroy()`) immediately on close or focus loss.
    Transient { destroy_on_blur: bool },
    /// Pinned resident window: stays alive while pinned on screen, destroyed on close.
    PinnedResident,
}

/// Declarative specification for spawning an on-demand WebView window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowSpec {
    pub label: String,
    pub title: String,
    /// Frontend route path (e.g., `"/capture-overlay"`, `"/dashboard/usage"`).
    pub route: String,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub width: f64,
    pub height: f64,
    pub min_width: Option<f64>,
    pub min_height: Option<f64>,
    pub resizable: bool,
    pub decorations: bool,
    pub transparent: bool,
    pub always_on_top: bool,
    pub skip_taskbar: bool,
    pub focus: bool,
    pub ignore_cursor_events: bool,
    pub policy: WindowLifecyclePolicy,
}

impl WindowSpec {
    /// Preset for Module 01: Fullscreen transparent screenshot & Snipaste smart-region overlay.
    pub fn capture_overlay(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            label: "capture-overlay".to_string(),
            title: "Capture Overlay".to_string(),
            route: "/capture-overlay".to_string(),
            x: Some(x),
            y: Some(y),
            width,
            height,
            min_width: None,
            min_height: None,
            resizable: false,
            decorations: false,
            transparent: true,
            always_on_top: true,
            skip_taskbar: true,
            focus: true,
            ignore_cursor_events: false,
            policy: WindowLifecyclePolicy::Transient {
                destroy_on_blur: false,
            },
        }
    }

    /// Preset for Module 01: Snipaste pinned floating image window (`pin-img-{id}`).
    pub fn pin_image(pin_id: &str, x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            label: format!("pin-img-{}", pin_id),
            title: "Pinned Image".to_string(),
            route: format!("/pin-image?id={}", pin_id),
            x: Some(x),
            y: Some(y),
            width,
            height,
            min_width: Some(32.0),
            min_height: Some(32.0),
            resizable: true,
            decorations: false,
            transparent: true,
            always_on_top: true,
            skip_taskbar: true,
            focus: false,
            ignore_cursor_events: false,
            policy: WindowLifecyclePolicy::PinnedResident,
        }
    }

    /// Preset for Module 02: Ditto-style quick-paste clipboard popup near cursor.
    pub fn clipboard_popup(x: f64, y: f64) -> Self {
        Self {
            label: "clipboard-popup".to_string(),
            title: "Clipboard History".to_string(),
            route: "/clipboard-popup".to_string(),
            x: Some(x),
            y: Some(y),
            width: 380.0,
            height: 480.0,
            min_width: Some(320.0),
            min_height: Some(360.0),
            resizable: false,
            decorations: false,
            transparent: true,
            always_on_top: true,
            skip_taskbar: true,
            focus: true,
            ignore_cursor_events: false,
            policy: WindowLifecyclePolicy::Transient {
                destroy_on_blur: true,
            },
        }
    }

    /// Preset for Module 03: gInk-style fullscreen screen ink canvas + floating capsule toolbar.
    pub fn screen_ink_pair(screen_w: f64, screen_h: f64) -> (Self, Self) {
        let canvas = Self {
            label: "screen-ink-canvas".to_string(),
            title: "Screen Ink Canvas".to_string(),
            route: "/screen-ink/canvas".to_string(),
            x: Some(0.0),
            y: Some(0.0),
            width: screen_w,
            height: screen_h,
            min_width: None,
            min_height: None,
            resizable: false,
            decorations: false,
            transparent: true,
            always_on_top: true,
            skip_taskbar: true,
            focus: true,
            ignore_cursor_events: false,
            policy: WindowLifecyclePolicy::Transient {
                destroy_on_blur: false,
            },
        };

        let toolbar = Self {
            label: "screen-ink-toolbar".to_string(),
            title: "Screen Ink Toolbar".to_string(),
            route: "/screen-ink/toolbar".to_string(),
            x: Some((screen_w - 420.0) / 2.0),
            y: Some(24.0),
            width: 420.0,
            height: 56.0,
            min_width: None,
            min_height: None,
            resizable: false,
            decorations: false,
            transparent: true,
            always_on_top: true,
            skip_taskbar: true,
            focus: false,
            ignore_cursor_events: false,
            policy: WindowLifecyclePolicy::Transient {
                destroy_on_blur: false,
            },
        };

        (canvas, toolbar)
    }

    /// Preset for Module 04: Pinned Markdown + LaTeX sticky note window (`sticky-note-{id}`).
    pub fn sticky_note(note_id: &str, x: Option<f64>, y: Option<f64>, w: f64, h: f64) -> Self {
        Self {
            label: format!("sticky-note-{}", note_id),
            title: "Sticky Note".to_string(),
            route: format!("/sticky-note?id={}", note_id),
            x,
            y,
            width: w,
            height: h,
            min_width: Some(220.0),
            min_height: Some(180.0),
            resizable: true,
            decorations: false,
            transparent: true,
            always_on_top: true,
            skip_taskbar: true,
            focus: true,
            ignore_cursor_events: false,
            policy: WindowLifecyclePolicy::PinnedResident,
        }
    }

    /// Preset for Module 07: Floating selection translation popover near cursor.
    pub fn translate_popover(x: f64, y: f64) -> Self {
        Self {
            label: "translate-popover".to_string(),
            title: "Translator".to_string(),
            route: "/translate-popover".to_string(),
            x: Some(x),
            y: Some(y),
            width: 360.0,
            height: 260.0,
            min_width: Some(280.0),
            min_height: Some(160.0),
            resizable: true,
            decorations: false,
            transparent: true,
            always_on_top: true,
            skip_taskbar: true,
            focus: true,
            ignore_cursor_events: false,
            policy: WindowLifecyclePolicy::Transient {
                destroy_on_blur: true,
            },
        }
    }

    /// Preset for Module 09: Fullscreen crosshair picker overlay (`window-pin-picker`).
    pub fn window_pin_picker(screen_w: f64, screen_h: f64) -> Self {
        Self {
            label: "window-pin-picker".to_string(),
            title: "Window Pin Crosshair Picker".to_string(),
            route: "/window-pin-picker".to_string(),
            x: Some(0.0),
            y: Some(0.0),
            width: screen_w,
            height: screen_h,
            min_width: None,
            min_height: None,
            resizable: false,
            decorations: false,
            transparent: true,
            always_on_top: true,
            skip_taskbar: true,
            focus: true,
            ignore_cursor_events: false,
            policy: WindowLifecyclePolicy::Transient {
                destroy_on_blur: true,
            },
        }
    }

    /// Preset for unified Dashboard window (Settings, App Usage, Scripts, Media Proc, Password Vault).
    pub fn dashboard(sub_route: &str) -> Self {
        let normalized_route = if sub_route.starts_with('/') {
            format!("/dashboard{}", sub_route)
        } else {
            format!("/dashboard/{}", sub_route)
        };
        Self {
            label: "dashboard".to_string(),
            title: "All-in-One Toolbox".to_string(),
            route: normalized_route,
            x: None,
            y: None,
            width: 1040.0,
            height: 700.0,
            min_width: Some(860.0),
            min_height: Some(560.0),
            resizable: true,
            decorations: true,
            transparent: false,
            always_on_top: false,
            skip_taskbar: false,
            focus: true,
            ignore_cursor_events: false,
            policy: WindowLifecyclePolicy::Transient {
                destroy_on_blur: false,
            },
        }
    }
}

/// Abstract backend trait implemented by `src-tauri` (wrapping `tauri::AppHandle`)
/// and by `HeadlessWindowBackend` for standalone kernel testing.
pub trait WindowBackend: Send + Sync + 'static {
    /// Creates or focuses a WebView window according to `spec`.
    fn create_or_focus_window(&self, spec: &WindowSpec) -> CoreResult<()>;
    /// Truly destroys the WebView window and releases its OS WebView2/WKWebView process.
    fn destroy_window(&self, label: &str) -> CoreResult<()>;
    /// Toggles mouse event passthrough (`set_ignore_cursor_events`) for screen ink or scroll capture.
    fn set_ignore_cursor_events(&self, label: &str, ignore: bool) -> CoreResult<()>;
    /// Updates the always-on-top state of an existing window.
    fn set_always_on_top(&self, label: &str, always_on_top: bool) -> CoreResult<()>;
}

/// Default headless backend used during unit tests or CLI inspection before Tauri attaches.
#[derive(Default)]
pub struct HeadlessWindowBackend;

impl WindowBackend for HeadlessWindowBackend {
    fn create_or_focus_window(&self, _spec: &WindowSpec) -> CoreResult<()> {
        Ok(())
    }

    fn destroy_window(&self, _label: &str) -> CoreResult<()> {
        Ok(())
    }

    fn set_ignore_cursor_events(&self, _label: &str, _ignore: bool) -> CoreResult<()> {
        Ok(())
    }

    fn set_always_on_top(&self, _label: &str, _always_on_top: bool) -> CoreResult<()> {
        Ok(())
    }
}

/// Runtime state of an active WebView tracked by `WindowPoolManager`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActiveWindowEntry {
    pub spec: WindowSpec,
    pub ignore_cursor_events: bool,
    pub always_on_top: bool,
}

/// Manages active WebView windows and enforces immediate process destruction on close/blur.
pub struct WindowPoolManager {
    backend: Arc<dyn WindowBackend>,
    active_windows: RwLock<HashMap<String, ActiveWindowEntry>>,
}

impl Default for WindowPoolManager {
    fn default() -> Self {
        Self::new(Arc::new(HeadlessWindowBackend))
    }
}

impl WindowPoolManager {
    pub fn new(backend: Arc<dyn WindowBackend>) -> Self {
        Self {
            backend,
            active_windows: RwLock::new(HashMap::new()),
        }
    }

    /// Spawns a WebView window on demand (or focuses and updates its route if already open).
    pub fn open_window(&self, spec: WindowSpec) -> CoreResult<()> {
        self.backend.create_or_focus_window(&spec)?;
        let entry = ActiveWindowEntry {
            ignore_cursor_events: spec.ignore_cursor_events,
            always_on_top: spec.always_on_top,
            spec: spec.clone(),
        };
        self.active_windows
            .write()
            .insert(spec.label.clone(), entry);
        info!(label = %spec.label, route = %spec.route, "Opened on-demand WebView window");
        Ok(())
    }

    /// Immediately destroys a WebView window and removes it from the active pool,
    /// freeing the underlying WebView2 / WKWebView memory.
    pub fn destroy_window(&self, label: &str) -> CoreResult<bool> {
        let removed = self.active_windows.write().remove(label);
        if removed.is_some() {
            self.backend.destroy_window(label)?;
            info!(label = %label, "Destroyed WebView window and reclaimed memory");
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Handles window blur (focus lost) events: automatically destroys `Transient { destroy_on_blur: true }`
    /// popups such as `clipboard-popup` and `translate-popover`.
    pub fn on_window_blur(&self, label: &str) -> CoreResult<bool> {
        let should_destroy = self
            .active_windows
            .read()
            .get(label)
            .map(|entry| {
                matches!(
                    entry.spec.policy,
                    WindowLifecyclePolicy::Transient {
                        destroy_on_blur: true
                    }
                )
            })
            .unwrap_or(false);

        if should_destroy {
            self.destroy_window(label)
        } else {
            Ok(false)
        }
    }

    /// Toggles mouse click-through (`ignore_cursor_events`) on an active window (used by Screen Ink and Scroll Capture).
    pub fn set_mouse_passthrough(&self, label: &str, passthrough: bool) -> CoreResult<()> {
        let mut active = self.active_windows.write();
        let entry = active
            .get_mut(label)
            .ok_or_else(|| CoreError::WindowLifecycle {
                label: label.to_string(),
                reason: "Window is not currently active".to_string(),
            })?;
        self.backend.set_ignore_cursor_events(label, passthrough)?;
        entry.ignore_cursor_events = passthrough;
        Ok(())
    }

    /// Updates the always-on-top flag of an active window.
    pub fn set_always_on_top(&self, label: &str, always_on_top: bool) -> CoreResult<()> {
        let mut active = self.active_windows.write();
        let entry = active
            .get_mut(label)
            .ok_or_else(|| CoreError::WindowLifecycle {
                label: label.to_string(),
                reason: "Window is not currently active".to_string(),
            })?;
        self.backend.set_always_on_top(label, always_on_top)?;
        entry.always_on_top = always_on_top;
        Ok(())
    }

    /// Destroys all currently open WebView windows during daemon shutdown.
    pub fn destroy_all(&self) -> CoreResult<usize> {
        let labels: Vec<String> = self.active_windows.read().keys().cloned().collect();
        let mut count = 0;
        for label in labels {
            if self.destroy_window(&label)? {
                count += 1;
            }
        }
        Ok(count)
    }

    /// Returns the number of currently alive WebView instances (0 during silent background daemon operation).
    pub fn active_window_count(&self) -> usize {
        self.active_windows.read().len()
    }

    /// Returns a snapshot of an active window's state by label.
    pub fn get_active_window(&self, label: &str) -> Option<ActiveWindowEntry> {
        self.active_windows.read().get(label).cloned()
    }

    /// Lists all active WebView windows sorted by label.
    pub fn list_active_windows(&self) -> Vec<ActiveWindowEntry> {
        let mut entries: Vec<ActiveWindowEntry> =
            self.active_windows.read().values().cloned().collect();
        entries.sort_by(|a, b| a.spec.label.cmp(&b.spec.label));
        entries
    }
}
