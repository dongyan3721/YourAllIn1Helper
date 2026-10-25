//! 剪贴板屏蔽租约单元测试：并发重叠、RAII 释放、到期失效、兼容槽隔离。
use super::*;
use crate::EventBus;

fn registry() -> Arc<ClipboardSuppressionRegistry> {
    Arc::new(ClipboardSuppressionRegistry::default())
}

#[test]
fn overlapping_leases_keep_suppression_until_the_last_release() {
    // 旧 AtomicBool 的缺陷场景：翻译先结束不能放开密码本仍在保护的窗口。
    let reg = registry();
    let translator = reg.acquire("translator", "simulated_copy", Duration::from_secs(5));
    let vault = reg.acquire("password_vault", "secret_copy", Duration::from_secs(30));
    assert!(reg.is_suppressed());
    drop(translator);
    assert!(reg.is_suppressed(), "vault lease must still suppress");
    assert_eq!(reg.holders().len(), 1);
    assert_eq!(reg.holders()[0].owner, "password_vault");
    vault.release();
    assert!(!reg.is_suppressed());
}

#[test]
fn lease_is_released_on_early_return_and_panic_unwind() {
    let reg = registry();
    fn guarded(reg: &Arc<ClipboardSuppressionRegistry>) -> Result<(), &'static str> {
        let _lease = reg.acquire("quick_paste", "paste", Duration::from_secs(5));
        Err("simulated failure")?;
        Ok(())
    }
    assert!(guarded(&reg).is_err());
    assert!(!reg.is_suppressed());

    let reg2 = Arc::clone(&reg);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _lease = reg2.acquire("translator", "panic", Duration::from_secs(5));
        panic!("simulated panic");
    }));
    assert!(result.is_err());
    assert!(!reg.is_suppressed());
}

#[test]
fn leaked_lease_expires_and_ttl_is_capped() {
    let reg = registry();
    std::mem::forget(reg.acquire("buggy", "forgotten", Duration::from_millis(20)));
    assert!(reg.is_suppressed());
    std::thread::sleep(Duration::from_millis(40));
    assert!(
        !reg.is_suppressed(),
        "leaked lease must not suppress forever"
    );

    let long = reg.acquire("x", "too long", Duration::from_secs(10_000));
    assert!(reg.holders()[0].remaining_ms <= MAX_LEASE_DURATION.as_millis() as u64);
    drop(long);
}

#[test]
fn extend_renews_live_lease_and_reports_expired_one() {
    let reg = registry();
    let lease = reg.acquire("vault", "clear_window", Duration::from_millis(20));
    assert!(lease.extend(Duration::from_secs(5)));
    std::thread::sleep(Duration::from_millis(40));
    assert!(reg.is_suppressed());
    let short = reg.acquire("translator", "short", Duration::from_millis(10));
    std::thread::sleep(Duration::from_millis(25));
    assert!(!short.extend(Duration::from_secs(1)));
    drop(lease);
    assert!(!reg.is_suppressed());
}

#[test]
fn legacy_switch_cannot_release_leases_and_leases_do_not_clear_legacy() {
    let reg = registry();
    let lease = reg.acquire("vault", "secret", Duration::from_secs(5));
    reg.set_legacy(false, "old caller done");
    assert!(reg.is_suppressed(), "legacy false must not drop a lease");
    reg.set_legacy(true, "old caller");
    drop(lease);
    assert!(
        reg.is_suppressed(),
        "lease release must not clear legacy slot"
    );
    reg.set_legacy(false, "old caller done");
    assert!(!reg.is_suppressed());
}

#[test]
fn event_bus_keeps_legacy_api_compatible_and_exposes_leases() {
    let bus = EventBus::default();
    bus.set_clipboard_suppression(true, "legacy");
    assert!(bus.is_clipboard_suppressed());
    let lease = bus.acquire_clipboard_suppression("vault", "copy", Duration::from_secs(5));
    bus.publish(crate::SystemEvent::SuppressClipboardCapture {
        suppressed: false,
        reason: "legacy done".into(),
    });
    assert!(bus.is_clipboard_suppressed());
    assert_eq!(bus.clipboard_suppression_holders().len(), 1);
    // 克隆的 EventBus 共享同一注册表。
    let clone = bus.clone();
    drop(lease);
    assert!(!clone.is_clipboard_suppressed());
}
