//! 剪贴板捕获屏蔽租约（带所有者、可并发、RAII 自动释放）。
//!
//! 旧实现只有一个 `AtomicBool`：翻译模拟复制与密码本复制重叠时，先结束的一方写入 `false`，
//! 会提前放开另一方仍需保护的剪贴板窗口。租约版本中：
//! - 每次屏蔽获得独立租约，持有期间 `is_suppressed()` 为真；全部租约释放后才解除屏蔽；
//! - 租约在 `Drop` 时释放，`?` 提前返回、panic 展开、任务取消都会归还；
//! - 每个租约有上限时长，即使被 `mem::forget` 泄漏也会到期失效，不会永久屏蔽剪贴板历史；
//! - 旧的 `set_clipboard_suppression(bool)` 接口保留为独立的「兼容槽」，只能开关自己，
//!   不能释放其他模块的租约。
use parking_lot::Mutex;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

/// 单个租约允许的最长时长；超过即视为失效。
pub const MAX_LEASE_DURATION: Duration = Duration::from_secs(300);

/// 当前持有的屏蔽来源（用于诊断显示，不含剪贴板内容）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SuppressionHolder {
    pub owner: String,
    pub reason: String,
    pub remaining_ms: u64,
}

#[derive(Debug)]
struct LeaseRecord {
    owner: String,
    reason: String,
    expires_at: Instant,
}

#[derive(Debug, Default)]
struct State {
    next_id: u64,
    leases: BTreeMap<u64, LeaseRecord>,
    /// 旧接口的兼容槽：`Some(reason)` 表示通过旧接口开启了屏蔽。
    legacy: Option<LeaseRecord>,
}

impl State {
    fn prune(&mut self, now: Instant) {
        self.leases.retain(|_, lease| lease.expires_at > now);
        if self.legacy.as_ref().is_some_and(|l| l.expires_at <= now) {
            self.legacy = None;
        }
    }
}

/// 屏蔽租约注册表，由 `EventBus` 持有并在模块间共享。
#[derive(Debug, Default)]
pub struct ClipboardSuppressionRegistry {
    state: Mutex<State>,
}

impl ClipboardSuppressionRegistry {
    /// 申请租约；`ttl` 被限制在 1 毫秒到 [`MAX_LEASE_DURATION`] 之间。
    pub fn acquire(
        self: &Arc<Self>,
        owner: impl Into<String>,
        reason: impl Into<String>,
        ttl: Duration,
    ) -> ClipboardSuppressionLease {
        let ttl = ttl.clamp(Duration::from_millis(1), MAX_LEASE_DURATION);
        let mut state = self.state.lock();
        state.next_id += 1;
        let id = state.next_id;
        state.leases.insert(
            id,
            LeaseRecord {
                owner: owner.into(),
                reason: reason.into(),
                expires_at: Instant::now() + ttl,
            },
        );
        ClipboardSuppressionLease {
            id,
            registry: Arc::clone(self),
            released: false,
        }
    }

    /// 是否存在任一有效租约或兼容槽。
    pub fn is_suppressed(&self) -> bool {
        let mut state = self.state.lock();
        state.prune(Instant::now());
        !state.leases.is_empty() || state.legacy.is_some()
    }

    /// 当前有效的屏蔽来源列表。
    pub fn holders(&self) -> Vec<SuppressionHolder> {
        let now = Instant::now();
        let mut state = self.state.lock();
        state.prune(now);
        let remaining =
            |l: &LeaseRecord| l.expires_at.saturating_duration_since(now).as_millis() as u64;
        state
            .leases
            .values()
            .chain(state.legacy.iter())
            .map(|l| SuppressionHolder {
                owner: l.owner.clone(),
                reason: l.reason.clone(),
                remaining_ms: remaining(l),
            })
            .collect()
    }

    /// 旧接口：只开关兼容槽，不影响任何租约。开启时同样受最长时长约束。
    pub fn set_legacy(&self, suppressed: bool, reason: &str) {
        let mut state = self.state.lock();
        state.legacy = suppressed.then(|| LeaseRecord {
            owner: "legacy".to_string(),
            reason: reason.to_string(),
            expires_at: Instant::now() + MAX_LEASE_DURATION,
        });
    }

    fn release(&self, id: u64) -> bool {
        self.state.lock().leases.remove(&id).is_some()
    }

    fn extend(&self, id: u64, ttl: Duration) -> bool {
        let ttl = ttl.clamp(Duration::from_millis(1), MAX_LEASE_DURATION);
        let now = Instant::now();
        let mut state = self.state.lock();
        // 已到期但尚未清理的租约不能被续期复活。
        state.prune(now);
        match state.leases.get_mut(&id) {
            Some(lease) => {
                lease.expires_at = now + ttl;
                true
            }
            None => false,
        }
    }
}

/// 屏蔽租约：持有期间剪贴板历史不记录新内容，离开作用域自动释放。
#[must_use = "租约离开作用域即释放屏蔽，请在需要保护的整个期间持有"]
#[derive(Debug)]
pub struct ClipboardSuppressionLease {
    id: u64,
    registry: Arc<ClipboardSuppressionRegistry>,
    released: bool,
}

impl ClipboardSuppressionLease {
    /// 续期（例如复制后的 30 秒清理窗口）；租约已过期返回 `false`，调用方应重新申请。
    pub fn extend(&self, ttl: Duration) -> bool {
        self.registry.extend(self.id, ttl)
    }

    /// 显式提前释放（等价于 drop）。
    pub fn release(mut self) {
        self.released = true;
        self.registry.release(self.id);
    }
}

impl Drop for ClipboardSuppressionLease {
    fn drop(&mut self) {
        if !self.released {
            self.registry.release(self.id);
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/clipboard_guard.rs"]
mod tests;
