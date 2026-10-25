use super::{ExitCoordinator, ExitDecision};
use std::sync::{Arc, Barrier};

#[test]
fn duplicate_requests_wait_and_reissued_exit_is_allowed_after_shutdown() {
    let exit = ExitCoordinator::default();
    assert!(!exit.is_shutting_down());
    assert_eq!(exit.request(), ExitDecision::StartShutdown);
    assert!(exit.is_shutting_down());
    assert_eq!(exit.request(), ExitDecision::Wait);
    assert_eq!(exit.request(), ExitDecision::Wait);
    exit.finish_shutdown();
    assert!(exit.is_shutting_down());
    assert_eq!(exit.request(), ExitDecision::Allow);
    assert_eq!(exit.request(), ExitDecision::Allow);
}

#[test]
fn concurrent_exit_sources_start_exactly_one_shutdown() {
    let exit = Arc::new(ExitCoordinator::default());
    let barrier = Arc::new(Barrier::new(8));
    let requests: Vec<_> = (0..8)
        .map(|_| {
            let exit = exit.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                exit.request()
            })
        })
        .collect();
    let decisions: Vec<_> = requests.into_iter().map(|r| r.join().unwrap()).collect();
    assert_eq!(
        decisions
            .iter()
            .filter(|d| **d == ExitDecision::StartShutdown)
            .count(),
        1
    );
    assert_eq!(
        decisions
            .iter()
            .filter(|d| **d == ExitDecision::Wait)
            .count(),
        7
    );
    exit.finish_shutdown();
    assert_eq!(exit.request(), ExitDecision::Allow);
}
