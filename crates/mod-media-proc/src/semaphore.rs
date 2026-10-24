use parking_lot::{Condvar, Mutex};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub struct JobSemaphore {
    running: Mutex<usize>,
    available: Condvar,
    limit: AtomicUsize,
}

impl JobSemaphore {
    pub fn new(limit: usize) -> Self {
        Self {
            running: Mutex::new(0),
            available: Condvar::new(),
            limit: AtomicUsize::new(limit),
        }
    }

    pub fn acquire(&self, cancelled: &AtomicBool) -> bool {
        let mut running = self.running.lock();
        while *running >= self.limit.load(Ordering::Acquire) {
            if cancelled.load(Ordering::Acquire) {
                return false;
            }
            self.available
                .wait_for(&mut running, std::time::Duration::from_millis(100));
        }
        if cancelled.load(Ordering::Acquire) {
            return false;
        }
        *running += 1;
        true
    }

    pub fn release(&self) {
        let mut running = self.running.lock();
        *running = running.saturating_sub(1);
        self.available.notify_one();
    }

    pub fn set_limit(&self, limit: usize) {
        self.limit.store(limit, Ordering::Release);
        self.wake_all();
    }

    pub fn wake_all(&self) {
        self.available.notify_all();
    }
}
