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

#[test]
fn unknown_state_and_failure_paths_are_conservative() {
    let state = UsageActivityState::default();

    // 初始状态下全部未知：轮询失败必须返回错误，上层暂停计费
    assert!(state.merge_screen_locked(failed()).is_err());

    // 仅收到解锁通知但唤醒源仍未知：轮询失败依然报错暂停
    state.apply(UsageActivityEvent::ScreenUnlocked);
    assert!(state.merge_screen_locked(failed()).is_err());

    // 唤醒源明确清醒后，解锁才允许在轮询失败时恢复
    state.apply(UsageActivityEvent::DidWake);
    assert_eq!(state.merge_screen_locked(failed()).unwrap(), false);

    // 重新进入睡眠后立即暂停
    state.apply(UsageActivityEvent::WillSleep);
    assert_eq!(state.merge_screen_locked(Ok(false)).unwrap(), true);
}

#[test]
fn out_of_order_and_rapid_state_transitions() {
    let state = UsageActivityState::default();

    // 模拟快速交替锁屏与解锁
    state.apply(UsageActivityEvent::ScreenLocked);
    assert_eq!(state.merge_screen_locked(Ok(false)).unwrap(), true);
    state.apply(UsageActivityEvent::ScreenUnlocked);
    assert_eq!(state.merge_screen_locked(Ok(false)).unwrap(), false);
    state.apply(UsageActivityEvent::ScreenLocked);
    assert_eq!(state.merge_screen_locked(Ok(false)).unwrap(), true);

    // 模拟屏幕息屏与亮屏
    state.apply(UsageActivityEvent::ScreensDidSleep);
    assert_eq!(state.merge_screen_locked(Ok(false)).unwrap(), true);
    state.apply(UsageActivityEvent::ScreensDidWake);
    // 此时虽然屏幕唤醒，但会话仍锁屏，结果仍为 true
    assert_eq!(state.merge_screen_locked(Ok(false)).unwrap(), true);
}

#[test]
fn state_reset_clears_stale_events_and_restores_unlocked_polling() {
    let state = UsageActivityState::default();

    // 1. 模拟先前处于锁屏状态
    state.apply(UsageActivityEvent::ScreenLocked);
    // 即使轮询报告未锁屏，由于陈旧锁屏事件存在，合并结果仍会被锁屏覆盖
    assert_eq!(state.merge_screen_locked(Ok(false)).unwrap(), true);

    // 2. 停止或重启时调用 reset() 清除陈旧事件状态
    state.reset();

    // 3. reset 之后，轮询报告未锁屏不再被陈旧锁屏遮蔽，能够准确恢复计费
    assert_eq!(state.merge_screen_locked(Ok(false)).unwrap(), false);

    // 4. reset 之后在无新事件且轮询失败时，必须保守保持未知错误，不得冒充已解锁
    assert!(state.merge_screen_locked(failed()).is_err());
}

#[test]
fn state_machine_restart_and_failure_recovery_pure_logic() {
    let state = UsageActivityState::default();

    // 模拟睡眠前状态
    state.apply(UsageActivityEvent::WillSleep);
    state.apply(UsageActivityEvent::ScreensDidSleep);
    assert_eq!(state.merge_screen_locked(Ok(false)).unwrap(), true);

    // 模拟监视器异常停止或失败重启：重置状态
    state.reset();
    assert!(state.merge_screen_locked(failed()).is_err());

    // 监视器重启后收到唤醒与解锁事件
    state.apply(UsageActivityEvent::DidWake);
    state.apply(UsageActivityEvent::ScreenUnlocked);

    // 此时即便轮询因偶发原因失败，也能通过权威唤醒与解锁事件确认清醒
    assert_eq!(state.merge_screen_locked(failed()).unwrap(), false);
}

#[cfg(target_os = "windows")]
#[test]
fn windows_screen_locked_decision_logic_covers_all_branches() {
    const WTS_SESSIONSTATE_LOCK: u32 = 0;
    const WTS_SESSIONSTATE_UNLOCK: u32 = 1;
    const WTS_SESSIONSTATE_UNKNOWN: u32 = 4294967295;

    // 1. SessionFlags 为明确锁定：无论桌面权限如何均判定为锁屏 (true)
    assert_eq!(
        evaluate_windows_screen_locked(Ok(WTS_SESSIONSTATE_LOCK), Ok(())).unwrap(),
        true
    );
    assert_eq!(
        evaluate_windows_screen_locked(
            Ok(WTS_SESSIONSTATE_LOCK),
            Err(HalError::PermissionDenied {
                capability: "desktop_switch",
                guidance: "restricted".to_string(),
            })
        )
        .unwrap(),
        true
    );

    // 2. SessionFlags 为解锁状态：
    // 2a. 桌面打开成功 -> 判定为未锁屏 (false)
    assert_eq!(
        evaluate_windows_screen_locked(Ok(WTS_SESSIONSTATE_UNLOCK), Ok(())).unwrap(),
        false
    );
    // 2b. 桌面打开报 PermissionDenied (Access Denied 5 / 0x80070005) -> 处于受限/凭据桌面，判定为锁屏 (true)
    assert_eq!(
        evaluate_windows_screen_locked(
            Ok(WTS_SESSIONSTATE_UNLOCK),
            Err(HalError::PermissionDenied {
                capability: "desktop_switch",
                guidance: "restricted".to_string(),
            })
        )
        .unwrap(),
        true
    );
    // 2c. 桌面打开遇到其它底层调用失败 -> 保守向上传播错误，不判定为未锁屏
    assert!(evaluate_windows_screen_locked(
        Ok(WTS_SESSIONSTATE_UNLOCK),
        Err(HalError::NativeCallFailed("desktop error".to_string()))
    )
    .is_err());

    // 3. SessionFlags 为未知状态 (0xFFFFFFFF 或其它未定义值) -> 无法可靠读取，必须报错
    assert!(evaluate_windows_screen_locked(Ok(WTS_SESSIONSTATE_UNKNOWN), Ok(())).is_err());
    assert!(evaluate_windows_screen_locked(Ok(999), Ok(())).is_err());

    // 4. WTS 查询本身失败 -> 保守向上传播错误
    assert!(evaluate_windows_screen_locked(
        Err(HalError::NativeCallFailed("WTS failed".to_string())),
        Ok(())
    )
    .is_err());
}

#[cfg(target_os = "windows")]
#[test]
fn windows_audio_active_decision_logic_covers_all_branches() {
    // 1. 任一探测明确检测到活跃音频 -> 判定为活跃 (true)
    assert_eq!(
        evaluate_windows_audio_active(Ok(true), Ok(true)).unwrap(),
        true
    );
    assert_eq!(
        evaluate_windows_audio_active(Ok(true), Ok(false)).unwrap(),
        true
    );
    assert_eq!(
        evaluate_windows_audio_active(
            Ok(true),
            Err(HalError::NativeCallFailed("meter error".to_string()))
        )
        .unwrap(),
        true
    );
    assert_eq!(
        evaluate_windows_audio_active(Ok(false), Ok(true)).unwrap(),
        true
    );
    assert_eq!(
        evaluate_windows_audio_active(
            Err(HalError::NativeCallFailed("session error".to_string())),
            Ok(true)
        )
        .unwrap(),
        true
    );

    // 2. 两路探测均成功且均未检测到音频活动 -> 判定为未活跃 (false)
    assert_eq!(
        evaluate_windows_audio_active(Ok(false), Ok(false)).unwrap(),
        false
    );

    // 3. 一路成功且无活跃，另一路失败（如硬件不支持电平计量）：信任成功的无活跃探测
    assert_eq!(
        evaluate_windows_audio_active(
            Ok(false),
            Err(HalError::NativeCallFailed("meter unsupported".to_string()))
        )
        .unwrap(),
        false
    );
    assert_eq!(
        evaluate_windows_audio_active(
            Err(HalError::NativeCallFailed("session unsupported".to_string())),
            Ok(false)
        )
        .unwrap(),
        false
    );

    // 4. 关键契约：两路探测全部失败时，严禁吞错冒充 false，必须返回真实错误！
    let both_failed = evaluate_windows_audio_active(
        Err(HalError::NativeCallFailed("session COM failed".to_string())),
        Err(HalError::NativeCallFailed("meter COM failed".to_string())),
    );
    assert!(both_failed.is_err());
    let err_msg = both_failed.unwrap_err().to_string();
    assert!(err_msg.contains("session COM failed") && err_msg.contains("meter COM failed"));
}
