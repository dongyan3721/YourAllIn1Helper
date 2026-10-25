//! Internal Publish-Subscribe EventBus for decoupled inter-module communication.
//!
//! Built on `tokio::sync::broadcast`, allowing global hooks (mouse selection,
//! active window watcher, idle timer) and modules (e.g., translator, password vault,
//! screenshot, clipboard) to communicate without tight coupling.

use crate::clipboard_guard::{
    ClipboardSuppressionLease, ClipboardSuppressionRegistry, SuppressionHolder,
};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};
use tokio::sync::broadcast;

pub use crate::screenshot_region::{
    infer_visual_edge_box_rgba, resolve_smart_region_at_point, DetectedRegionRect,
    RegionHierarchyLevel, ResizeHandle, SelectionBox,
};

/// Screenshot capture sub-modes triggered via shortcut or tray.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMode {
    /// Standard Snipaste-style region capture (supports smart bounding-box detection + manual drag box selection).
    Standard,
    /// QQ NT-style scrolling long screenshot stitching.
    ScrollLong,
    /// Multi-resolution GIF recording.
    GifRecording,
    /// Capture region and immediately invoke OCR pipeline.
    ImmediateOcr,
}

/// System-wide domain events broadcast across `app-core` and the 10 feature modules.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type", content = "payload", rename_all = "snake_case")]
pub enum SystemEvent {
    /// Instructs `mod-clipboard` to temporarily ignore clipboard mutations.
    /// Emitted by `mod-translator` (simulated Ctrl+C fallback) and `mod-password-vault` (secret copy).
    SuppressClipboardCapture { suppressed: bool, reason: String },

    /// Emitted by `mod-clipboard` when a new item is persisted in the LRU-100 table.
    ClipboardUpdated { item_id: i64, item_type: String },

    /// Emitted when a global mouse drag selection ends (subscribed by `mod-translator`).
    MouseSelectionEnded {
        x: i32,
        y: i32,
        selected_text_hint: Option<String>,
    },

    /// Emitted when the OS foreground active window changes (subscribed by `mod-app-usage` and `mod-window-pin`).
    ForegroundAppChanged {
        app_id: String,
        app_name: String,
        window_title: String,
        process_id: u32,
    },

    /// Emitted when system idle/AFK or screen lock state changes (subscribed by `mod-app-usage` and `mod-password-vault`).
    SystemIdleStateChanged { is_idle: bool, idle_seconds: u64 },

    /// Emitted when screenshot overlay is requested (`mod-screenshot`).
    ScreenCaptureRequested {
        mode: CaptureMode,
        enable_smart_region_detection: bool,
    },

    /// Emitted by `mod-password-vault` when the vault is locked or unlocked.
    VaultLockStateChanged { locked: bool },

    /// Emitted when `AppConfig` is updated and hot-reloaded.
    ConfigReloaded,

    /// Extensible custom topic event for module-to-module notifications.
    CustomModuleEvent {
        source_module: String,
        topic: String,
        payload_json: String,
    },
}

/// Thread-safe internal event bus shared via `ModuleContext`.
#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<SystemEvent>,
    clipboard_suppression: Arc<ClipboardSuppressionRegistry>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(256)
    }
}

impl EventBus {
    /// Creates a new `EventBus` with the specified ring-buffer capacity.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.max(16));
        Self {
            sender,
            clipboard_suppression: Arc::new(ClipboardSuppressionRegistry::default()),
        }
    }

    /// Publishes a `SystemEvent` to all active subscribers.
    /// `SuppressClipboardCapture` 只开关旧接口的兼容槽，不会释放其他模块持有的租约。
    pub fn publish(&self, event: SystemEvent) -> usize {
        if let SystemEvent::SuppressClipboardCapture { suppressed, reason } = &event {
            self.clipboard_suppression.set_legacy(*suppressed, reason);
        }
        // If there are no active receivers, `send` returns Err, which is harmless.
        self.sender.send(event).unwrap_or(0)
    }

    /// Subscribes to the event bus, returning a `broadcast::Receiver<SystemEvent>`.
    pub fn subscribe(&self) -> broadcast::Receiver<SystemEvent> {
        self.sender.subscribe()
    }

    /// `mod-clipboard` 记录新内容前检查：任一有效租约或兼容槽存在即为真。
    #[inline]
    pub fn is_clipboard_suppressed(&self) -> bool {
        self.clipboard_suppression.is_suppressed()
    }

    /// 申请剪贴板捕获屏蔽租约（推荐接口）：持有期间不记录剪贴板历史，离开作用域自动释放，
    /// `ttl` 到期也会失效（上限 [`crate::clipboard_guard::MAX_LEASE_DURATION`]）。
    pub fn acquire_clipboard_suppression(
        &self,
        owner: impl Into<String>,
        reason: impl Into<String>,
        ttl: Duration,
    ) -> ClipboardSuppressionLease {
        self.clipboard_suppression.acquire(owner, reason, ttl)
    }

    /// 当前屏蔽来源（诊断用，不含剪贴板内容）。
    pub fn clipboard_suppression_holders(&self) -> Vec<SuppressionHolder> {
        self.clipboard_suppression.holders()
    }

    /// 旧接口：开关兼容槽并广播事件。新代码请使用 [`Self::acquire_clipboard_suppression`]。
    pub fn set_clipboard_suppression(&self, suppressed: bool, reason: impl Into<String>) {
        self.publish(SystemEvent::SuppressClipboardCapture {
            suppressed,
            reason: reason.into(),
        });
    }
}
