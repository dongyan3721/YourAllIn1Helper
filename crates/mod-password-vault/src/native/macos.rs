use crate::{error::VaultError, LockReason, VaultResult, VaultService};
use block2::RcBlock;
use objc2::{MainThreadMarker, Message};
use objc2_app_kit::{
    NSWorkspace, NSWorkspaceScreensDidSleepNotification,
    NSWorkspaceSessionDidResignActiveNotification, NSWorkspaceWillSleepNotification,
};
use objc2_foundation::{
    NSDistributedNotificationCenter, NSNotification, NSNotificationCenter, NSNotificationName,
    NSOperationQueue, NSString,
};
use std::{ptr::NonNull, sync::Arc};

/// 只在 AppKit 主线程注册/销毁。观察者与密码本业务隔离，事件载荷不进入服务。
pub struct NativeVaultObserver {
    removals: Vec<Box<dyn FnOnce()>>,
}

impl NativeVaultObserver {
    pub fn start(service: Arc<VaultService>) -> VaultResult<Self> {
        MainThreadMarker::new().ok_or(VaultError::FeatureDisabled(
            "native vault observer requires the AppKit main thread",
        ))?;
        let mut observer = Self {
            removals: Vec::new(),
        };
        let workspace = NSWorkspace::sharedWorkspace();
        let center = workspace.notificationCenter();
        // SAFETY：这些是 AppKit 提供的静态通知名称，生命周期覆盖整个进程。
        let notifications = unsafe {
            [
                (NSWorkspaceWillSleepNotification, LockReason::Sleep),
                (NSWorkspaceScreensDidSleepNotification, LockReason::Sleep),
                (
                    NSWorkspaceSessionDidResignActiveNotification,
                    LockReason::ScreenLock,
                ),
            ]
        };
        for (name, reason) in notifications {
            observer.observe_workspace(&center, name, reason, Arc::clone(&service));
        }

        let distributed = NSDistributedNotificationCenter::defaultCenter();
        observer.observe_distributed(
            &distributed,
            &NSString::from_str("com.apple.screenIsLocked"),
            Arc::clone(&service),
        );
        Ok(observer)
    }

    fn observe_workspace(
        &mut self,
        center: &NSNotificationCenter,
        name: &NSNotificationName,
        reason: LockReason,
        service: Arc<VaultService>,
    ) {
        let queue = NSOperationQueue::mainQueue();
        let block = RcBlock::new(move |_notification: NonNull<NSNotification>| {
            service.lock(reason);
        });
        // SAFETY：block 不读取通知载荷；queue 固定为主队列，token 在 stop 时成对移除。
        let token = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(name),
                None,
                Some(&queue),
                &block,
            )
        };
        let center = center.retain();
        self.removals.push(Box::new(move || {
            // SAFETY：token 来自同一个 notification center，且只移除一次。
            let observer: &objc2::runtime::AnyObject = (&*token).as_ref();
            unsafe { center.removeObserver(observer) };
        }));
    }

    fn observe_distributed(
        &mut self,
        center: &NSDistributedNotificationCenter,
        name: &NSString,
        service: Arc<VaultService>,
    ) {
        let queue = NSOperationQueue::mainQueue();
        let block = RcBlock::new(move |_notification: NonNull<NSNotification>| {
            service.lock(LockReason::ScreenLock);
        });
        // SAFETY：与工作区通知相同；分布式通知仅使用固定名称，不接收或记录秘密。
        let token = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(name),
                None,
                Some(&queue),
                &block,
            )
        };
        let center = center.retain();
        self.removals.push(Box::new(move || {
            // SAFETY：token 与 center 配对，stop 在主线程调用且只执行一次。
            let observer: &objc2::runtime::AnyObject = (&*token).as_ref();
            unsafe { center.removeObserver(observer) };
        }));
    }

    pub fn stop(mut self) {
        assert!(
            MainThreadMarker::new().is_some(),
            "NativeVaultObserver::stop must run on the AppKit main thread"
        );
        self.remove_all();
    }

    fn remove_all(&mut self) {
        for remove in self.removals.drain(..) {
            remove();
        }
    }
}

impl Drop for NativeVaultObserver {
    fn drop(&mut self) {
        if MainThreadMarker::new().is_some() {
            self.remove_all();
        }
        // 非主线程误 Drop 时让 center 持有 block/token，避免跨线程 AppKit 调用和 UAF；
        // 生产接线必须显式在主线程 stop，不能依赖此降级。
    }
}
