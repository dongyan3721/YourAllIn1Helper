//! Native exit requests share one shutdown transaction while the UI event loop stays live.
use std::sync::atomic::{AtomicU8, Ordering};

const RUNNING: u8 = 0;
const SHUTTING_DOWN: u8 = 1;
const READY_TO_EXIT: u8 = 2;

#[derive(Default)]
pub(crate) struct ExitCoordinator(AtomicU8);

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ExitDecision {
    StartShutdown,
    Wait,
    Allow,
}

impl ExitCoordinator {
    pub(crate) fn is_shutting_down(&self) -> bool {
        self.0.load(Ordering::Acquire) != RUNNING
    }

    pub(crate) fn request(&self) -> ExitDecision {
        match self
            .0
            .compare_exchange(RUNNING, SHUTTING_DOWN, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => ExitDecision::StartShutdown,
            Err(READY_TO_EXIT) => ExitDecision::Allow,
            Err(_) => ExitDecision::Wait,
        }
    }

    pub(crate) fn finish_shutdown(&self) {
        self.0.store(READY_TO_EXIT, Ordering::Release);
    }
}

#[cfg(test)]
#[path = "../tests/unit/exit_coordinator.rs"]
mod tests;
