use super::*;
use crate::HalError;

fn failed() -> HalResult<bool> {
    Err(HalError::NativeCallFailed(
        "session unavailable".to_string(),
    ))
}

#[test]
fn system_events_pause_and_only_resume_after_confirmed_unlock() {
    let state = UsageActivityState::default();

    // 没有事件且轮询失败时保持未知，不得当作活跃。
    assert!(state.merge_screen_locked(failed()).is_err());

    // 锁屏通知优先于仍返回 Ok(false) 的轮询（事件与轮询不一致时取暂停）。
    state.apply(UsageActivityEvent::ScreenLocked);
    assert_eq!(state.merge_screen_locked(Ok(false)).unwrap(), true);

    state.apply(UsageActivityEvent::ScreensDidSleep);
    assert_eq!(state.merge_screen_locked(Ok(false)).unwrap(), true);

    // 只是屏幕唤醒还不够：会话仍锁定，轮询失败时必须继续暂停。
    state.apply(UsageActivityEvent::ScreensDidWake);
    assert_eq!(state.merge_screen_locked(failed()).unwrap(), true);

    // 明确解锁后恢复；轮询失败也可由通知确认。
    state.apply(UsageActivityEvent::ScreenUnlocked);
    assert_eq!(state.merge_screen_locked(failed()).unwrap(), false);
}

#[test]
fn sleep_wake_requires_both_workspace_and_screen_wake() {
    let state = UsageActivityState::default();
    state.apply(UsageActivityEvent::WillSleep);
    state.apply(UsageActivityEvent::ScreensDidSleep);
    state.apply(UsageActivityEvent::ScreenLocked);

    state.apply(UsageActivityEvent::DidWake);
    // 只收到 willSleep/didWake 时屏幕仍处于睡眠，继续暂停。
    assert_eq!(state.merge_screen_locked(failed()).unwrap(), true);

    state.apply(UsageActivityEvent::ScreensDidWake);
    // 屏幕已醒但会话仍锁定，暂停。
    assert_eq!(state.merge_screen_locked(Ok(false)).unwrap(), true);

    state.apply(UsageActivityEvent::ScreenUnlocked);
    assert_eq!(state.merge_screen_locked(failed()).unwrap(), false);
}

#[test]
fn polling_result_remains_authoritative_when_events_are_incomplete() {
    let state = UsageActivityState::default();
    assert_eq!(state.merge_screen_locked(Ok(false)).unwrap(), false);
    assert_eq!(state.merge_screen_locked(Ok(true)).unwrap(), true);
}
