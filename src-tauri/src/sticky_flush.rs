//! Page-confirmation gate for destroying sticky-note windows during shutdown.
//!
//! The module asks the native backend to flush a page; the page responds through the
//! `sticky_flush_result` IPC command. Waiters use a condition variable with an absolute
//! deadline, so there is no sleep polling and a missing page response cannot block forever.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

use app_core::CloseFlushOutcome;

#[derive(Default)]
pub(crate) struct FlushSignal {
    result: Mutex<Option<Result<(), String>>>,
    ready: Condvar,
}

/// Coordinates shutdown flush requests with page IPC confirmations.
pub(crate) struct StickyFlushCoordinator {
    main_thread: ThreadId,
    waiters: Mutex<HashMap<String, Arc<FlushSignal>>>,
}

impl Default for StickyFlushCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl StickyFlushCoordinator {
    pub(crate) fn new() -> Self {
        Self {
            main_thread: thread::current().id(),
            waiters: Mutex::new(HashMap::new()),
        }
    }

    /// A synchronous wait on the native main thread would deadlock WebView IPC delivery.
    pub(crate) fn is_main_thread(&self) -> bool {
        thread::current().id() == self.main_thread
    }

    pub(crate) fn register(&self, note_id: &str) -> Arc<FlushSignal> {
        let signal = Arc::new(FlushSignal::default());
        self.waiters
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(note_id.to_string(), signal.clone());
        signal
    }

    pub(crate) fn cancel(&self, note_id: &str, signal: &Arc<FlushSignal>) {
        let mut waiters = self.waiters.lock().unwrap_or_else(|err| err.into_inner());
        let remove = waiters
            .get(note_id)
            .is_some_and(|current| Arc::ptr_eq(current, signal));
        if remove {
            waiters.remove(note_id);
        }
    }

    /// Resolves a page confirmation. Returns false when no shutdown flush is waiting.
    pub(crate) fn complete(&self, note_id: &str, result: Result<(), String>) -> bool {
        let signal = self
            .waiters
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(note_id);
        let Some(signal) = signal else {
            return false;
        };
        *signal.result.lock().unwrap_or_else(|err| err.into_inner()) = Some(result);
        signal.ready.notify_all();
        true
    }

    pub(crate) fn wait(
        &self,
        note_id: &str,
        signal: Arc<FlushSignal>,
        timeout: Duration,
    ) -> CloseFlushOutcome {
        let deadline = Instant::now() + timeout;
        let mut result = signal.result.lock().unwrap_or_else(|err| err.into_inner());
        loop {
            if let Some(result) = result.take() {
                return match result {
                    Ok(()) => CloseFlushOutcome::Saved,
                    Err(reason) => CloseFlushOutcome::Failed(reason),
                };
            }
            let now = Instant::now();
            if now >= deadline {
                drop(result);
                self.cancel(note_id, &signal);
                return CloseFlushOutcome::TimedOut;
            }
            let (next, timed_out) = signal
                .ready
                .wait_timeout(result, deadline - now)
                .unwrap_or_else(|err| err.into_inner());
            result = next;
            if timed_out.timed_out() && result.is_none() {
                drop(result);
                self.cancel(note_id, &signal);
                return CloseFlushOutcome::TimedOut;
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/sticky_flush.rs"]
mod tests;
