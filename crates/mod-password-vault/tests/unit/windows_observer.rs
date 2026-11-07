//! 会话/睡眠观察者。只向本进程隐藏窗口发送消息，不调用 LockWorkStation，不睡眠，不弹认证框。
use crate::{native::windows, LockReason, VaultService};
use std::sync::Arc;

#[test]
fn abandoned_observer_start_releases_its_service_and_window() {
    let service = Arc::new(VaultService::with_random(Arc::new(
        super::support::CountingRandom::new(),
    )));
    windows::abandoned_start_for_test(Arc::clone(&service));
    assert_eq!(Arc::strong_count(&service), 1);
}

#[test]
fn session_and_power_codes_match_the_documented_values() {
    assert_eq!(7, 0x7);
    assert_eq!(
        windows::session_lock_reason(7),
        Some(LockReason::ScreenLock)
    );
    assert_eq!(
        windows::session_lock_reason(2),
        Some(LockReason::ScreenLock)
    );
    assert_eq!(
        windows::session_lock_reason(4),
        Some(LockReason::ScreenLock)
    );
    assert_eq!(
        windows::session_lock_reason(6),
        Some(LockReason::ScreenLock)
    );
    assert_eq!(windows::session_lock_reason(8), None);
    assert_eq!(windows::session_lock_reason(1), None);
    assert_eq!(windows::power_lock_reason(4), Some(LockReason::Sleep));
    assert_eq!(windows::power_lock_reason(7), None);
    assert_eq!(windows::power_lock_reason(18), None);
    assert_eq!(
        windows::window_message_reason(17, 0),
        Some(LockReason::ScreenLock)
    );
    assert_eq!(
        windows::window_message_reason(22, 1),
        Some(LockReason::ScreenLock)
    );
    assert_eq!(windows::window_message_reason(22, 0), None);
}

#[test]
fn observer_locks_on_injected_events_and_can_restart() {
    let service = Arc::new(VaultService::with_random(Arc::new(
        super::support::CountingRandom::new(),
    )));
    let observer = windows::NativeVaultObserver::start(Arc::clone(&service)).expect("start");
    let hwnd = observer.hwnd().expect("hwnd");
    let before = service.status().epoch;
    windows::inject_for_test(hwnd, 689, 7);
    assert!(service.status().epoch > before);
    assert!(service.status().locked);

    let after_lock = service.status().epoch;
    windows::inject_for_test(hwnd, 689, 8);
    assert_eq!(service.status().epoch, after_lock);
    windows::inject_for_test(hwnd, 536, 4);
    let after_sleep = service.status().epoch;
    assert!(after_sleep > after_lock);
    windows::inject_for_test(hwnd, 536, 7);
    windows::inject_for_test(hwnd, 536, 18);
    assert_eq!(service.status().epoch, after_sleep);

    observer.stop();
    let again = windows::NativeVaultObserver::start(Arc::clone(&service)).expect("restart");
    let epoch = service.status().epoch;
    windows::inject_for_test(again.hwnd().expect("hwnd"), 689, 2);
    assert!(service.status().epoch > epoch);
    drop(again);
    let third = windows::NativeVaultObserver::start(service).expect("after drop");
    third.stop();
}
