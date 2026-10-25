//! macOS 锁屏 / 息屏 / 睡眠事件观察者。
//!
//! 使用独立后台线程及其 RunLoop 接收 `NSWorkspace` 与分布式通知，避免占用
//! AppKit 主线程；观察者只更新原子状态，业务模块通过 `ActiveAppWatcher` 读取。
//! 停止时撤销两个通知中心的观察者并等待线程退出，重复启停不会累积观察者。

use super::{UsageActivityEvent, UsageActivityState};
use crate::{HalError, HalResult};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{define_class, msg_send, sel, AnyThread, DefinedClass};
use objc2_app_kit::{
    NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceScreensDidSleepNotification,
    NSWorkspaceScreensDidWakeNotification, NSWorkspaceWillSleepNotification,
};
use objc2_core_foundation::{kCFRunLoopDefaultMode, CFRunLoop};
use objc2_foundation::{
    NSDistributedNotificationCenter, NSNotification, NSNotificationCenter, NSString,
};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};

#[derive(Clone)]
struct ObserverIvars {
    state: Arc<UsageActivityState>,
}

define_class!(
    // SAFETY: NSObject 没有额外子类要求，观察者不实现 Drop，且只在事件线程内存活。
    #[unsafe(super(NSObject))]
    #[name = "AioUsageNotificationObserver"]
    #[ivars = ObserverIvars]
    struct UsageNotificationObserver;

    impl UsageNotificationObserver {
        #[unsafe(method(workspaceWillSleep:))]
        fn workspace_will_sleep(&self, _notification: &NSNotification) {
            self.ivars().state.apply(UsageActivityEvent::WillSleep);
        }

        #[unsafe(method(workspaceDidWake:))]
        fn workspace_did_wake(&self, _notification: &NSNotification) {
            self.ivars().state.apply(UsageActivityEvent::DidWake);
        }

        #[unsafe(method(screensDidSleep:))]
        fn screens_did_sleep(&self, _notification: &NSNotification) {
            self.ivars().state.apply(UsageActivityEvent::ScreensDidSleep);
        }

        #[unsafe(method(screensDidWake:))]
        fn screens_did_wake(&self, _notification: &NSNotification) {
            self.ivars().state.apply(UsageActivityEvent::ScreensDidWake);
        }

        #[unsafe(method(screenLocked:))]
        fn screen_locked(&self, _notification: &NSNotification) {
            self.ivars().state.apply(UsageActivityEvent::ScreenLocked);
        }

        #[unsafe(method(screenUnlocked:))]
        fn screen_unlocked(&self, _notification: &NSNotification) {
            self.ivars()
                .state
                .apply(UsageActivityEvent::ScreenUnlocked);
        }
    }
);

impl UsageNotificationObserver {
    fn new(state: Arc<UsageActivityState>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ObserverIvars { state });
        unsafe { msg_send![super(this), init] }
    }

    /// 交给 Objective-C 通知中心保存的裸对象视图。
    fn as_object(&self) -> &AnyObject {
        unsafe { &*(self as *const Self as *const AnyObject) }
    }
}

/// 事件线程句柄；`running` 控制 RunLoop 轮询退出的时机。
pub(super) struct MacUsageEventMonitor {
    state: Arc<UsageActivityState>,
    running: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl MacUsageEventMonitor {
    pub(super) fn new(state: Arc<UsageActivityState>) -> Self {
        Self {
            state,
            running: Arc::new(AtomicBool::new(false)),
            thread: Mutex::new(None),
        }
    }

    pub(super) fn start(&self) -> HalResult<()> {
        let mut guard = self.thread.lock();
        if guard.as_ref().is_some_and(|handle| !handle.is_finished()) {
            return Ok(());
        }
        if let Some(handle) = guard.take() {
            let _ = handle.join();
        }

        self.running.store(true, Ordering::SeqCst);
        let (ready_tx, ready_rx) = mpsc::sync_channel::<HalResult<()>>(1);
        let state = Arc::clone(&self.state);
        let running = Arc::clone(&self.running);
        let handle = thread::Builder::new()
            .name("app-usage-events".to_string())
            .spawn(move || run_monitor(state, running, ready_tx))
            .map_err(|error| {
                self.running.store(false, Ordering::SeqCst);
                HalError::NativeCallFailed(format!(
                    "failed to spawn macOS usage event thread: {error}"
                ))
            })?;

        match ready_rx.recv() {
            Ok(Ok(())) => {
                *guard = Some(handle);
                Ok(())
            }
            Ok(Err(error)) => {
                let _ = handle.join();
                Err(error)
            }
            Err(_) => {
                let _ = handle.join();
                Err(HalError::NativeCallFailed(
                    "macOS usage event thread exited before ready".to_string(),
                ))
            }
        }
    }

    pub(super) fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.thread.lock().take() {
            let _ = handle.join();
        }
    }
}

impl Drop for MacUsageEventMonitor {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run_monitor(
    state: Arc<UsageActivityState>,
    running: Arc<AtomicBool>,
    ready: mpsc::SyncSender<HalResult<()>>,
) {
    let Some(run_loop) = CFRunLoop::current() else {
        let _ = ready.send(Err(HalError::NativeCallFailed(
            "macOS usage event thread has no CFRunLoop".to_string(),
        )));
        running.store(false, Ordering::SeqCst);
        return;
    };

    let observer = UsageNotificationObserver::new(state);
    let workspace_center: Retained<NSNotificationCenter> =
        NSWorkspace::sharedWorkspace().notificationCenter();
    let distributed_center = NSDistributedNotificationCenter::defaultCenter();
    let locked_name = NSString::from_str("com.apple.screenIsLocked");
    let unlocked_name = NSString::from_str("com.apple.screenIsUnlocked");

    unsafe {
        for (name, selector) in [
            (NSWorkspaceWillSleepNotification, sel!(workspaceWillSleep:)),
            (NSWorkspaceDidWakeNotification, sel!(workspaceDidWake:)),
            (
                NSWorkspaceScreensDidSleepNotification,
                sel!(screensDidSleep:),
            ),
            (NSWorkspaceScreensDidWakeNotification, sel!(screensDidWake:)),
        ] {
            workspace_center.addObserver_selector_name_object(
                observer.as_object(),
                selector,
                Some(name),
                None,
            );
        }
        distributed_center.addObserver_selector_name_object(
            observer.as_object(),
            sel!(screenLocked:),
            Some(&locked_name),
            None,
        );
        distributed_center.addObserver_selector_name_object(
            observer.as_object(),
            sel!(screenUnlocked:),
            Some(&unlocked_name),
            None,
        );
    }

    let _ = ready.send(Ok(()));

    // 限时运行 RunLoop，使线程能响应停止标志；通知本身仍由系统异步派发。
    let default_mode = unsafe { kCFRunLoopDefaultMode };
    while running.load(Ordering::SeqCst) {
        CFRunLoop::run_in_mode(default_mode, 0.1, false);
    }

    unsafe {
        workspace_center.removeObserver(observer.as_object());
        distributed_center.removeObserver(observer.as_object());
    }
    running.store(false, Ordering::SeqCst);
    drop(run_loop);
}
