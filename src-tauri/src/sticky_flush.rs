//! Page-confirmation gate for destroying sticky-note windows during shutdown.
//!
//! The module asks the native backend to flush a page; the page responds through the
//! `sticky_flush_result` IPC command. Waiters use a condition variable with an absolute
//! deadline, so there is no sleep polling and a missing page response cannot block forever.
//! Every attempt has a unique request id, so a late acknowledgement for a timed-out
//! attempt cannot release a newer attempt for the same note.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

use app_core::CloseFlushOutcome;

#[derive(Default)]
pub(crate) struct FlushSignal {
    result: Mutex<Option<Result<(), String>>>,
    ready: Condvar,
}

struct FlushWaiter {
    request_id: u64,
    signal: Arc<FlushSignal>,
}

/// Coordinates shutdown flush requests with page IPC confirmations.
pub(crate) struct StickyFlushCoordinator {
    main_thread: ThreadId,
    next_request: AtomicU64,
    waiters: Mutex<HashMap<String, FlushWaiter>>,
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
            next_request: AtomicU64::new(0),
            waiters: Mutex::new(HashMap::new()),
        }
    }

    /// A synchronous wait on the native main thread would deadlock WebView IPC delivery.
    pub(crate) fn is_main_thread(&self) -> bool {
        thread::current().id() == self.main_thread
    }

    /// Starts a new flush attempt with a request id that is unique across retries.
    pub(crate) fn begin(&self, note_id: &str) -> (u64, Arc<FlushSignal>) {
        let request_id = self.next_request.fetch_add(1, Ordering::Relaxed) + 1;
        let signal = Arc::new(FlushSignal::default());
        self.waiters
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(
                note_id.to_string(),
                FlushWaiter {
                    request_id,
                    signal: signal.clone(),
                },
            );
        (request_id, signal)
    }

    pub(crate) fn cancel(&self, note_id: &str, request_id: u64, signal: &Arc<FlushSignal>) {
        let mut waiters = self.waiters.lock().unwrap_or_else(|err| err.into_inner());
        let remove = waiters.get(note_id).is_some_and(|current| {
            current.request_id == request_id && Arc::ptr_eq(&current.signal, signal)
        });
        if remove {
            waiters.remove(note_id);
        }
    }

    /// Resolves a page confirmation only when its request id matches the active attempt.
    pub(crate) fn complete(
        &self,
        note_id: &str,
        request_id: u64,
        result: Result<(), String>,
    ) -> bool {
        let signal = {
            let mut waiters = self.waiters.lock().unwrap_or_else(|err| err.into_inner());
            if waiters
                .get(note_id)
                .is_none_or(|current| current.request_id != request_id)
            {
                return false;
            }
            waiters.remove(note_id).map(|waiter| waiter.signal)
        };
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
        request_id: u64,
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
                self.cancel(note_id, request_id, &signal);
                return CloseFlushOutcome::TimedOut;
            }
            let (next, timed_out) = signal
                .ready
                .wait_timeout(result, deadline - now)
                .unwrap_or_else(|err| err.into_inner());
            result = next;
            if timed_out.timed_out() && result.is_none() {
                drop(result);
                self.cancel(note_id, request_id, &signal);
                return CloseFlushOutcome::TimedOut;
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/sticky_flush.rs"]
mod tests;
