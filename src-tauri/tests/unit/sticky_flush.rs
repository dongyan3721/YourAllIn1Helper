use super::*;

#[test]
fn page_confirmation_resolves_waiter_with_success() {
    let coordinator = Arc::new(StickyFlushCoordinator::new());
    let signal = coordinator.register("note-a");
    let waiter = {
        let coordinator = coordinator.clone();
        let signal = signal.clone();
        std::thread::spawn(move || coordinator.wait("note-a", signal, Duration::from_secs(2)))
    };
    assert!(coordinator.complete("note-a", Ok(())));
    assert_eq!(waiter.join().unwrap(), CloseFlushOutcome::Saved);
    assert!(!coordinator.complete("note-a", Ok(())));
}

#[test]
fn page_failure_is_reported_and_keeps_the_window_decision_with_the_caller() {
    let coordinator = StickyFlushCoordinator::new();
    let signal = coordinator.register("note-b");
    assert!(coordinator.complete("note-b", Err("disk full".to_string())));
    assert_eq!(
        coordinator.wait("note-b", signal, Duration::from_secs(1)),
        CloseFlushOutcome::Failed("disk full".to_string())
    );
}

#[test]
fn missing_page_response_times_out_without_polling_the_clock_forever() {
    let coordinator = StickyFlushCoordinator::new();
    let signal = coordinator.register("note-c");
    let started = Instant::now();
    assert_eq!(
        coordinator.wait("note-c", signal, Duration::from_millis(30)),
        CloseFlushOutcome::TimedOut
    );
    assert!(started.elapsed() >= Duration::from_millis(25));
    assert!(started.elapsed() < Duration::from_secs(1));
    // The timed-out waiter is removed so a late page response cannot leak state.
    assert!(!coordinator.complete("note-c", Ok(())));
}

#[test]
fn late_confirmation_after_timeout_is_not_acknowledged() {
    let coordinator = StickyFlushCoordinator::new();
    let signal = coordinator.register("note-d");
    assert_eq!(
        coordinator.wait("note-d", signal, Duration::from_millis(10)),
        CloseFlushOutcome::TimedOut
    );
    assert!(!coordinator.complete("note-d", Ok(())));
}

#[test]
fn unknown_or_duplicate_acknowledgements_are_rejected() {
    let coordinator = StickyFlushCoordinator::new();
    assert!(!coordinator.complete("missing", Ok(())));
    let signal = coordinator.register("note-e");
    assert!(coordinator.complete("note-e", Ok(())));
    assert!(!coordinator.complete("note-e", Ok(())));
    drop(signal);
}
